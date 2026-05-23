# Plugin Manifest Format (M4.4 + dynamic loader follow-up)

> **Alpha plugin-safety warning.** In 0.3.0, third-party plugins are **not**
> recommended for end users. The plugin ABI is a raw `extern "Rust"` symbol
> handshake with no version check, so a plugin built against a different
> strivo build or a different rustc toolchain can silently corrupt memory
> and crash the daemon — taking any in-flight recording with it. Run only
> first-party plugins (Crunchr, Archiver) shipped from the matching
> `Chorosyne/strivo-plugins` revision, unless you are the plugin author and
> compiled it against this exact strivo checkout. A versioned ABI handshake
> is tracked for 0.4.x.

Drop a TOML file at
`~/.config/strivo/plugins/<slug>.toml` and StriVo will discover it on
startup, list it in the Settings tab, and — if `library_path` is set —
dlopen the named cdylib at daemon launch.

## Fields

```toml
name              = "scratchpad"
version           = "0.1.0"
description       = "Quick-notes scratchpad pinned to ,s"
activation_letter = "s"                # preferred — registers as ",s"
# activation_key  = "F2"               # deprecated; pre-comma-namespace form
pane              = "right"            # "right" | "overlay" | "statusbar"
library_path      = "~/scratchpad.so"  # cdylib loaded via libloading
```

All fields except `name` are optional. Unknown fields are ignored (so
future StriVo versions can extend the schema without breaking older
manifests).

### Plugin keybinding namespace (`,X`)

As of v0.4, plugins should register their activation key as a single
letter under the **`,` (comma) plugin leader** — `,c` for Crunchr,
`,a` for Archiver, `,s` for the scratchpad example above. Set
`activation_letter = "<one letter>"`; the host wires it to
`<comma><letter>` automatically.

This keeps plugin activations out of the global keymap so they cannot
collide with built-in bindings like `s` (settings) or `a` (toggle
auto-record). Two plugins claiming the same letter under `,` will
log a warning on startup and only the first registered will fire —
pick a different letter to fix it.

The older `activation_key` field still works but is **deprecated**
and logs an info-level migration nudge on startup. Migrate at your
convenience; both fields can coexist while you transition.

## How discovery works

- `~/.config/strivo/plugins/` is scanned at AppState construction.
- Each `*.toml` file is read; parse errors are logged and skipped.
- Successfully parsed manifests appear in **Settings → Plugins** as
  rows showing `<name>  v<version> · <activation_key>` plus the
  description as the hint.
- Files without a `.toml` extension are ignored.
- Missing directory: silently no-ops. Drop a manifest in to enable.
- If `library_path` is present, the daemon dlopens the library at
  startup, calls the registration symbol, and registers the returned
  `Box<dyn Plugin>` alongside the first-party plugins (Crunchr,
  Archiver).

## Writing a dynamic plugin

Your plugin is a regular Cargo crate compiled as `cdylib`:

```toml
# Cargo.toml
[package]
name = "strivo-scratchpad"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
strivo-core = { path = "/path/to/strivo" }  # SAME build as host
anyhow = "1"
```

Implement the `strivo_core::plugin::Plugin` trait, then export the
registration symbol:

```rust
use strivo_core::plugin::{Plugin, /* … */};

pub struct Scratchpad { /* … */ }

impl Plugin for Scratchpad { /* … */ }

/// MUST be named exactly `_strivo_plugin_register` and have this
/// signature. Returns ownership to the host via a leaked
/// `Box<Box<dyn Plugin>>`.
#[no_mangle]
pub extern "C" fn _strivo_plugin_register() -> *mut std::ffi::c_void {
    let plugin: Box<dyn Plugin> = Box::new(Scratchpad::new());
    Box::into_raw(Box::new(plugin)) as *mut std::ffi::c_void
}
```

Build with `cargo build --release`; the resulting
`target/release/libstrivo_scratchpad.so` (or `.dylib` / `.dll`) is
what you point `library_path` at.

## Critical caveat: same-toolchain only

Rust's `dyn Trait` vtable layout is **not stable across compilation
units that don't share an exact dependency closure**. That means your
plugin cdylib MUST be compiled with:

- The same `rustc` version as the StriVo host binary.
- The exact same `strivo-core` build (same git revision / same path
  dep / same Cargo.lock entries).
- The same set of feature flags on every transitively-used crate.

Violating this is undefined behavior — at best you get cryptic
crashes inside the registry, at worst silent memory corruption.

In practice this means **plugins are not portable binaries**. Ship
them as source or as an installer that compiles from source against
the user's StriVo build.

The pragmatic alternative (`abi_stable` or `stabby` for true ABI-
stable plugins) is tracked as a future hardening item; the current
loader is sufficient for in-organization plugins and for
experimenters.

## Limits

- **`activation_key` is advisory.** The keymap table doesn't bind
  manifest activation keys automatically — the plugin still wires
  its activation via `Plugin::commands()`. Collisions log a warning
  at startup (`audit_manifest_conflicts`).
- **First-party plugins** (Crunchr, Archiver) compile in via the path
  dep and don't need manifests.
- **No hot reload.** Plugins load once at daemon start.

## Loading sequence

```
daemon::run()
 └─ scan_user_plugins(user_plugin_dir())
     ├─ audit_manifest_conflicts(&manifests)
     └─ PluginRegistry::load_dylibs_from_manifests(&manifests)
         └─ for each manifest with library_path:
             ├─ libloading::Library::new(path)
             ├─ library.get(b"_strivo_plugin_register")
             ├─ symbol() → *mut c_void
             ├─ cast back to Box<Box<dyn Plugin>>, unwrap one layer
             ├─ registry.register_dylib(LoadedDylibPlugin {
             │       plugin,
             │       library,        // kept alive in loaded_libraries
             │   })
             └─ registry.init_all() runs all plugin::init() hooks
```

Failures (missing file, missing symbol, registration returns null)
log a warning and skip the entry; the other plugins keep loading.

## Companion docs

- `YAZI-AUDIT.md` §5 — original audit + scope decision (no Lua).
- `ROADMAP.md` M4 Phase 4 — plugin manifest + discovery item.
- `src/plugin/mod.rs` — `Plugin` trait, `load_dylib_plugin`,
  `PLUGIN_REGISTER_SYMBOL`.
- `src/plugin/registry.rs` — `register_dylib`,
  `load_dylibs_from_manifests`.
