use std::any::Any;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::{AppState, DaemonEvent};
use crate::config::AppConfig;

use super::{DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext};

/// Lifecycle state for a registered plugin. Surfaced by the plugin browser
/// (P1) and consulted by `init_all` / `shutdown_all` to report errors
/// without aborting the whole pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Registered but `init` has not yet run.
    Initializing,
    /// `init` succeeded; plugin is operational.
    Ready,
    /// `init` or `shutdown` returned an error. Surfaced in the plugin
    /// browser and the event log.
    Error(String),
    /// Plugin is registered but suppressed (currently reserved — no UI
    /// path sets this yet).
    Disabled,
}

#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    /// Per-plugin lifecycle state, parallel-indexed with `plugins`.
    statuses: Vec<PluginStatus>,
    pane_map: HashMap<PaneId, usize>,
    /// O(1) lookup for plugin activation keybindings.
    command_map: HashMap<(KeyCode, KeyModifiers), PaneId>,
    active_plugin_pane: Option<PaneId>,
    /// `libloading::Library` handles for dynamically-loaded plugins.
    /// MUST outlive the corresponding `Box<dyn Plugin>` — the vtable
    /// lives in the loaded image. Dropped together when the registry
    /// drops, so order matters: plugins drop first (we own them in
    /// `plugins`), then the libraries.
    loaded_libraries: Vec<libloading::Library>,
    /// DAW-vision capability → plugin index. Populated at register
    /// time so `providers_of(cap)` is an O(1) lookup. See
    /// `crate::plugin::capability` for the well-known strings.
    capability_index: HashMap<&'static str, Vec<usize>>,
}


impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let idx = self.plugins.len();
        // Build pane map
        for pane_id in plugin.panes() {
            self.pane_map.insert(pane_id, idx);
        }
        // Build command map for O(1) keybinding lookup
        for cmd in plugin.commands() {
            if let Some(pane) = plugin.panes().into_iter().next() {
                self.command_map.insert((cmd.key, cmd.modifiers), pane);
            }
        }
        // Index DAW-vision capabilities so the host can ask
        // `providers_of(cap)` without re-iterating every plugin's
        // capabilities() each time. Same idea as pane_map: O(plugins)
        // at register time, O(1) at query time.
        for cap in plugin.capabilities() {
            self.capability_index.entry(cap).or_default().push(idx);
        }
        self.plugins.push(plugin);
        self.statuses.push(PluginStatus::Initializing);
    }

    /// Plugin names that declare a given capability tag.
    pub fn providers_of(&self, capability: &str) -> Vec<&str> {
        self.capability_index
            .get(capability)
            .map(|idxs| idxs.iter().map(|i| self.plugins[*i].name()).collect())
            .unwrap_or_default()
    }

    /// Every (capability, providers) tuple — used by the SPA's plugin
    /// hub to render the cross-plugin graph.
    pub fn capability_map(&self) -> std::collections::BTreeMap<&str, Vec<&str>> {
        let mut out = std::collections::BTreeMap::new();
        for (cap, idxs) in &self.capability_index {
            out.insert(*cap, idxs.iter().map(|i| self.plugins[*i].name()).collect());
        }
        out
    }

    /// Register a dynamically-loaded plugin. The library MUST outlive
    /// the plugin — the registry holds the `libloading::Library` for
    /// its lifetime to guarantee that.
    pub fn register_dylib(&mut self, loaded: super::LoadedDylibPlugin) {
        self.register(loaded.plugin);
        // Library drops AFTER all plugins because we own them in
        // separate Vecs and Rust drops fields in declaration order:
        // `plugins` declared first → dropped first → vtable still
        // resolvable. Then `loaded_libraries` drops.
        self.loaded_libraries.push(loaded.library);
    }

    /// Scan and load every manifest in `manifests` whose `library_path`
    /// is set. Manifests with no library_path are informational
    /// (M4.4 surface) and skipped here. Returns the count of plugins
    /// successfully loaded; failures are logged.
    pub fn load_dylibs_from_manifests(&mut self, manifests: &[super::PluginManifest]) -> usize {
        let mut loaded = 0;
        for m in manifests {
            let Some(ref lib_path) = m.library_path else {
                continue;
            };
            // Expand a leading ~ if present (the manifest is human-
            // edited; users naturally write paths that way).
            let expanded = match lib_path.to_str() {
                Some(s) if s.starts_with("~/") => {
                    if let Some(home) = std::env::var_os("HOME") {
                        std::path::PathBuf::from(home).join(&s[2..])
                    } else {
                        lib_path.clone()
                    }
                }
                _ => lib_path.clone(),
            };
            match super::load_dylib_plugin(&expanded) {
                Ok(plugin) => {
                    tracing::info!(
                        plugin = %m.name,
                        path = %expanded.display(),
                        "dynamic plugin loaded",
                    );
                    self.register_dylib(plugin);
                    loaded += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        plugin = %m.name,
                        path = %expanded.display(),
                        error = %e,
                        "dynamic plugin load failed",
                    );
                }
            }
        }
        loaded
    }

    /// Number of plugins registered. (W2 phase 2.)
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether any plugins are registered. (W2 phase 2.)
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn init_all(&mut self, config: &AppConfig) -> anyhow::Result<()> {
        let base_data = AppConfig::data_dir();
        let base_cache = AppConfig::cache_dir();

        for (idx, plugin) in self.plugins.iter_mut().enumerate() {
            let ctx = PluginContext {
                config,
                data_dir: base_data.join("plugins").join(plugin.name()),
                cache_dir: base_cache.join("plugins").join(plugin.name()),
            };
            match plugin.init(&ctx) {
                Ok(()) => {
                    if let Some(s) = self.statuses.get_mut(idx) {
                        *s = PluginStatus::Ready;
                    }
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    if let Some(s) = self.statuses.get_mut(idx) {
                        *s = PluginStatus::Error(msg.clone());
                    }
                    tracing::error!(plugin = %plugin.name(), error = %msg, "plugin init failed");
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub fn shutdown_all(&mut self) {
        for (idx, plugin) in self.plugins.iter_mut().enumerate() {
            let name = plugin.name();
            if let Err(e) = plugin.shutdown() {
                let msg = format!("{e:#}");
                if let Some(s) = self.statuses.get_mut(idx) {
                    *s = PluginStatus::Error(msg.clone());
                }
                tracing::error!(plugin = %name, error = %msg, "plugin shutdown failed");
            }
        }
    }

    /// Snapshot of each plugin's `(name, display_name, status)` for the
    /// plugin browser (P1) and diagnostics.
    pub fn plugin_statuses(&self) -> Vec<(&str, &str, &PluginStatus)> {
        self.plugins
            .iter()
            .zip(self.statuses.iter())
            .map(|(p, s)| (p.name(), p.display_name(), s))
            .collect()
    }

    /// Dispatch a DaemonEvent to all interested plugins.
    pub fn dispatch_event(&mut self, event: &DaemonEvent, app: &AppState) -> Vec<PluginAction> {
        let kind = DaemonEventKind::from_event(event);
        let mut actions = Vec::new();

        for plugin in &mut self.plugins {
            let interested = match plugin.event_filter() {
                None => true,
                Some(ref kinds) => kinds.contains(&kind),
            };
            if interested {
                actions.extend(plugin.on_event(event, app));
            }
        }
        actions
    }

    /// Dispatch a key event to the plugin owning the active pane.
    pub fn dispatch_key(&mut self, key: KeyEvent, app: &AppState) -> Vec<PluginAction> {
        if let Some(pane_id) = self.active_plugin_pane {
            if let Some(&idx) = self.pane_map.get(pane_id) {
                return self.plugins[idx].on_key(key, app);
            }
        }
        Vec::new()
    }

    /// Dispatch a custom plugin event to the named plugin.
    pub fn dispatch_plugin_event(
        &mut self,
        plugin_name: &str,
        event: Box<dyn Any + Send>,
    ) -> Vec<PluginAction> {
        for plugin in &mut self.plugins {
            if plugin.name() == plugin_name {
                return plugin.on_plugin_event(event);
            }
        }
        Vec::new()
    }

    /// Dispatch an actions-popup verb to its owning plugin. (M2.)
    pub fn dispatch_verb(
        &mut self,
        plugin_name: &str,
        verb: &str,
        selection: &[uuid::Uuid],
        ctx: &crate::plugin::VerbContext,
    ) -> Vec<PluginAction> {
        for plugin in &mut self.plugins {
            if plugin.name() == plugin_name {
                return plugin.on_verb(verb, selection, ctx);
            }
        }
        Vec::new()
    }

    /// Render the active plugin pane.
    pub fn render_active_pane(&self, frame: &mut Frame, area: Rect, app: &AppState) {
        if let Some(pane_id) = self.active_plugin_pane {
            if let Some(&idx) = self.pane_map.get(pane_id) {
                self.plugins[idx].render_pane(pane_id, frame, area, app);
            }
        }
    }

    pub fn set_active_pane(&mut self, pane_id: Option<PaneId>) {
        self.active_plugin_pane = pane_id;
    }

    /// Collect all plugin commands for the help overlay.
    pub fn all_commands(&self) -> Vec<(&'static str, PluginCommand)> {
        let mut cmds = Vec::new();
        for plugin in &self.plugins {
            for cmd in plugin.commands() {
                cmds.push((plugin.name(), cmd));
            }
        }
        cmds
    }

    /// Plugin verbs scoped to a specific item type. Backs the actions
    /// popup (D5+X5) — each entry becomes a row alongside built-in
    /// verbs like Play / Properties / Delete.
    pub fn item_commands(
        &self,
        kind: super::ItemKind,
    ) -> Vec<(&'static str, PluginCommand)> {
        use super::PluginCommandScope;
        let mut out = Vec::new();
        for plugin in &self.plugins {
            for cmd in plugin.commands() {
                if matches!(cmd.scope, PluginCommandScope::Item(k) if k == kind) {
                    out.push((plugin.name(), cmd));
                }
            }
        }
        out
    }

    /// Collect status line contributions from all plugins whose
    /// [`StatusSlot`] is `Tray` or `Banner` (banner falls back to tray
    /// until the M4 telemetry strip lands). Plugins that return
    /// `StatusSlot::None` are skipped even if `status_line` returned
    /// `Some`.
    pub fn status_lines(&self, app: &AppState) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|p| !matches!(p.status_slot(), crate::plugin::StatusSlot::None))
            .filter_map(|p| p.status_line(app))
            .collect()
    }

    /// O(1) lookup: find the pane ID for a command matching the given key event.
    pub fn pane_for_command(&self, key: &KeyEvent) -> Option<PaneId> {
        self.command_map.get(&(key.code, key.modifiers)).copied()
    }

    /// Collect properties-panel contributions from all plugins for the given
    /// recording job. Each plugin decides whether to emit anything.
    pub fn properties_sections(
        &self,
        job_id: uuid::Uuid,
        app: &AppState,
    ) -> Vec<ratatui::text::Line<'static>> {
        let mut out = Vec::new();
        for plugin in &self.plugins {
            let lines = plugin.properties_section(job_id, app);
            if !lines.is_empty() {
                out.extend(lines);
            }
        }
        out
    }

    /// Same data as [`properties_sections`] but grouped by plugin name, so the
    /// renderer can prepend a chip-style header per source. Plugins that
    /// contribute nothing are filtered out. Preserves registration order
    /// (first-party plugins registered first will sort first; third-party
    /// follow).
    pub fn properties_sections_grouped(
        &self,
        job_id: uuid::Uuid,
        app: &AppState,
    ) -> Vec<(String, Vec<ratatui::text::Line<'static>>)> {
        self.plugins
            .iter()
            .filter_map(|plugin| {
                let lines = plugin.properties_section(job_id, app);
                if lines.is_empty() {
                    None
                } else {
                    Some((plugin.name().to_string(), lines))
                }
            })
            .collect()
    }

    /// Look up a plugin by name for downcasting via `as_any()`.
    pub fn plugin_ref(&self, name: &str) -> Option<&dyn Plugin> {
        self.plugins
            .iter()
            .find(|p| p.name() == name)
            .map(|p| p.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Minimal test plugin for registry tests.
    struct TestPlugin {
        name: &'static str,
        filter: Option<Vec<DaemonEventKind>>,
        pane: Option<PaneId>,
    }

    impl TestPlugin {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                filter: None,
                pane: None,
            }
        }
        fn with_pane(mut self, pane: PaneId) -> Self {
            self.pane = Some(pane);
            self
        }
        #[allow(dead_code)]
        fn with_filter(mut self, kinds: Vec<DaemonEventKind>) -> Self {
            self.filter = Some(kinds);
            self
        }
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &'static str {
            self.name
        }
        fn display_name(&self) -> &str {
            self.name
        }
        fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
            Ok(())
        }
        fn event_filter(&self) -> Option<Vec<DaemonEventKind>> {
            self.filter.clone()
        }
        fn panes(&self) -> Vec<PaneId> {
            self.pane.into_iter().collect()
        }
        fn commands(&self) -> Vec<PluginCommand> {
            if self.pane.is_some() {
                vec![PluginCommand::new(
                    "test",
                    "test command",
                    KeyCode::Char('T'),
                    KeyModifiers::SHIFT,
                )]
            } else {
                Vec::new()
            }
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn register_populates_pane_and_command_maps() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("p1").with_pane("pane1")));

        assert!(reg.pane_map.contains_key("pane1"));
        assert_eq!(
            reg.command_map
                .get(&(KeyCode::Char('T'), KeyModifiers::SHIFT)),
            Some(&"pane1")
        );
    }

    #[test]
    fn pane_for_command_returns_none_on_no_match() {
        let reg = PluginRegistry::new();
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(reg.pane_for_command(&key).is_none());
    }

    #[test]
    fn pane_for_command_returns_pane_on_match() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("p1").with_pane("my_pane")));

        let key = KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT);
        assert_eq!(reg.pane_for_command(&key), Some("my_pane"));
    }

    #[test]
    fn dispatch_key_returns_empty_when_no_active_pane() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("p1").with_pane("pane1")));

        let key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let config = crate::config::AppConfig::default();
        let app = AppState::new(config);
        let actions = reg.dispatch_key(key, &app);
        assert!(actions.is_empty());
    }

    #[test]
    fn status_lines_collects_from_plugins() {
        let reg = PluginRegistry::new();
        let config = crate::config::AppConfig::default();
        let app = AppState::new(config);
        let lines = reg.status_lines(&app);
        assert!(lines.is_empty());
    }
}
