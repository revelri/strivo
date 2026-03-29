use std::any::Any;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::{AppState, DaemonEvent};
use crate::config::AppConfig;

use super::{DaemonEventKind, PaneId, Plugin, PluginAction, PluginCommand, PluginContext};

pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    pane_map: HashMap<PaneId, usize>,
    /// O(1) lookup for plugin activation keybindings.
    command_map: HashMap<(KeyCode, KeyModifiers), PaneId>,
    active_plugin_pane: Option<PaneId>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            pane_map: HashMap::new(),
            command_map: HashMap::new(),
            active_plugin_pane: None,
        }
    }
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
        self.plugins.push(plugin);
    }

    pub fn init_all(&mut self, config: &AppConfig) -> anyhow::Result<()> {
        let base_data = AppConfig::data_dir();
        let base_cache = AppConfig::cache_dir();

        for plugin in &mut self.plugins {
            let ctx = PluginContext {
                config,
                data_dir: base_data.join("plugins").join(plugin.name()),
                cache_dir: base_cache.join("plugins").join(plugin.name()),
            };
            plugin.init(&ctx)?;
        }
        Ok(())
    }

    pub fn shutdown_all(&mut self) {
        for plugin in &mut self.plugins {
            plugin.shutdown();
        }
    }

    /// Dispatch a DaemonEvent to all interested plugins.
    pub fn dispatch_event(
        &mut self,
        event: &DaemonEvent,
        app: &AppState,
    ) -> Vec<PluginAction> {
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
    pub fn dispatch_key(
        &mut self,
        key: KeyEvent,
        app: &AppState,
    ) -> Vec<PluginAction> {
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

    /// Collect status line contributions from all plugins.
    pub fn status_lines(&self, app: &AppState) -> Vec<String> {
        self.plugins
            .iter()
            .filter_map(|p| p.status_line(app))
            .collect()
    }

    /// O(1) lookup: find the pane ID for a command matching the given key event.
    pub fn pane_for_command(&self, key: &KeyEvent) -> Option<PaneId> {
        self.command_map.get(&(key.code, key.modifiers)).copied()
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
            Self { name, filter: None, pane: None }
        }
        fn with_pane(mut self, pane: PaneId) -> Self {
            self.pane = Some(pane);
            self
        }
        fn with_filter(mut self, kinds: Vec<DaemonEventKind>) -> Self {
            self.filter = Some(kinds);
            self
        }
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &'static str { self.name }
        fn display_name(&self) -> &str { self.name }
        fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> { Ok(()) }
        fn event_filter(&self) -> Option<Vec<DaemonEventKind>> { self.filter.clone() }
        fn panes(&self) -> Vec<PaneId> {
            self.pane.into_iter().collect()
        }
        fn commands(&self) -> Vec<PluginCommand> {
            if self.pane.is_some() {
                vec![PluginCommand {
                    name: "test",
                    description: "test command",
                    key: KeyCode::Char('T'),
                    modifiers: KeyModifiers::SHIFT,
                }]
            } else {
                Vec::new()
            }
        }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    fn register_populates_pane_and_command_maps() {
        let mut reg = PluginRegistry::new();
        reg.register(Box::new(TestPlugin::new("p1").with_pane("pane1")));

        assert!(reg.pane_map.contains_key("pane1"));
        assert_eq!(
            reg.command_map.get(&(KeyCode::Char('T'), KeyModifiers::SHIFT)),
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
