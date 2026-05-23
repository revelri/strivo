# Writing a plugin

> **Alpha.** The plugin ABI is unstable and unversioned in 0.3.0. A plugin
> built against a different strivo build (or a different rustc toolchain)
> can corrupt memory and crash the daemon. End users should not run
> third-party plugins yet — see [PLUGIN-MANIFEST.md](./PLUGIN-MANIFEST.md).
> If you are the plugin author and building against this exact checkout,
> read on.

A strivo plugin is a Rust `cdylib` that exports a single registration
symbol and a manifest TOML file that tells the daemon where to load it.

## Minimal example

### `Cargo.toml`

```toml
[package]
name    = "hello-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
strivo-core = { path = "../strivo" }   # must match the host's strivo-core
anyhow      = "1"
async-trait = "0.1"
tracing     = "0.1"
```

### `src/lib.rs`

```rust
use async_trait::async_trait;
use strivo_core::plugin::{Plugin, PluginContext, PluginRegistrar};

#[derive(Default)]
struct HelloPlugin;

#[async_trait]
impl Plugin for HelloPlugin {
    fn name(&self) -> &'static str {
        "hello-plugin"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    async fn on_recording_finished(
        &self,
        ctx: &PluginContext,
        job_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        tracing::info!(%job_id, "hello-plugin saw a finished recording");
        let _ = ctx;
        Ok(())
    }
}

/// Required entry point. The daemon `dlsym`s this symbol after dlopen and
/// calls it once per plugin instance.
#[no_mangle]
pub extern "Rust" fn _strivo_plugin_register(reg: &mut dyn PluginRegistrar) {
    reg.register(Box::new(HelloPlugin::default()));
}
```

### Manifest at `~/.config/strivo/plugins/hello.toml`

```toml
name           = "hello-plugin"
version        = "0.1.0"
description    = "Logs a line every time a recording finishes"
library_path   = "~/code/hello-plugin/target/release/libhello_plugin.so"
```

## Build and load

```bash
cargo build --release
strivo daemon restart        # picks up the new manifest
strivo log tail | grep hello
```

The daemon scans `~/.config/strivo/plugins/*.toml` on startup. Unparseable
manifests are logged at WARN and skipped — they never abort the daemon.
Successfully loaded plugins show up in **Settings → Plugins**.

## Things to know

- **One `strivo-core` per process.** Your plugin's `Cargo.toml` must point
  to the *same* `strivo-core` checkout the strivo binary was built from.
  The workspace `[patch]` block at the repo root keeps this honest for
  in-tree plugin development.
- **Same rustc.** The ABI of `Box<dyn Plugin>` is not stable across rustc
  versions. Build the plugin and strivo with the same toolchain (the
  `rust-toolchain.toml` file in this repo pins it).
- **Async runtime.** strivo embeds tokio; do not start your own runtime
  inside the plugin. Use the tokio handle on `PluginContext`.
- **No panics across the FFI boundary.** Wrap your handler with
  `tokio::task::spawn_blocking` if you must call panicking code; convert
  panics to `anyhow::Error` before returning.
- **Logging.** Use `tracing` macros; the daemon's subscriber will pick
  them up.
