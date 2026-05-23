use askama::Template;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::Router;

use strivo_core::config::AppConfig;
use strivo_core::ipc;

use crate::server::AppState;

#[derive(Template)]
#[template(
    source = r#"<section class="status-page">
  <h1>System status</h1>

  <h2>Daemon</h2>
  <ul>
    <li>State: <strong>{{ daemon_state }}</strong></li>
    {% if let Some(pid) = daemon_pid %}<li>PID: {{ pid }}</li>{% endif %}
    <li>Socket: <code>{{ socket_path }}</code></li>
    {% if let Some(channels) = channel_count %}<li>Channels tracked: {{ channels }}</li>{% endif %}
    {% if let Some(recs) = recording_count %}<li>Recordings tracked: {{ recs }}</li>{% endif %}
  </ul>

  <h2>Storage</h2>
  <ul>
    <li>Recording dir: <code>{{ recording_dir }}</code></li>
    <li>Writable: <strong>{{ recording_dir_writable }}</strong></li>
    {% if let Some(g) = disk_free_gib %}<li>Free space: {{ g }} GiB</li>{% endif %}
  </ul>

  <h2>Plugins</h2>
  {% if plugins.is_empty() %}
    <p>No user plugin manifests in <code>{{ plugin_dir }}</code>.</p>
  {% else %}
    <ul>
      {% for p in plugins %}<li><strong>{{ p.name }}</strong>{% if let Some(v) = p.version %} v{{ v }}{% endif %}{% if let Some(lib) = p.library %} — <code>{{ lib }}</code>{% endif %}</li>{% endfor %}
    </ul>
  {% endif %}
</section>
"#,
    ext = "html"
)]
struct StatusTemplate {
    daemon_state: &'static str,
    daemon_pid: Option<String>,
    socket_path: String,
    channel_count: Option<usize>,
    recording_count: Option<usize>,
    recording_dir: String,
    recording_dir_writable: &'static str,
    disk_free_gib: Option<u64>,
    plugin_dir: String,
    plugins: Vec<PluginRow>,
}

struct PluginRow {
    name: String,
    version: Option<String>,
    library: Option<String>,
}

async fn status(State(state): State<AppState>) -> Result<Html<String>, axum::http::StatusCode> {
    let daemon_running = ipc::is_daemon_running();
    let daemon_state = if daemon_running { "running" } else { "not running" };
    let daemon_pid = std::fs::read_to_string(ipc::pid_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Snapshot daemon for counts. Best-effort — if the daemon is down
    // we just leave the counts blank.
    let (channel_count, recording_count) = if daemon_running {
        match state.ipc.snapshot().await {
            Ok(strivo_core::ipc::ServerMessage::StateSnapshot {
                channels, recordings, ..
            }) => (Some(channels.len()), Some(recordings.len())),
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    let cfg = AppConfig::load(None).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let rec_dir = cfg.recording_dir.clone();
    let writable_test = rec_dir.exists() && {
        let probe = rec_dir.join(".strivo-web-write-probe");
        let ok = std::fs::write(&probe, b"ok").is_ok();
        let _ = std::fs::remove_file(&probe);
        ok
    };
    let recording_dir_writable = if writable_test { "yes" } else { "no" };
    let disk_free_gib = disk_free_bytes(&rec_dir).map(|b| b / (1024 * 1024 * 1024));

    let plugin_dir = strivo_core::plugin::user_plugin_dir();
    let manifests = strivo_core::plugin::scan_user_plugins(&plugin_dir);
    let plugins = manifests
        .into_iter()
        .map(|m| PluginRow {
            name: m.name,
            version: m.version,
            library: m.library_path.map(|p| p.display().to_string()),
        })
        .collect();

    let tpl = StatusTemplate {
        daemon_state,
        daemon_pid,
        socket_path: ipc::socket_path().display().to_string(),
        channel_count,
        recording_count,
        recording_dir: rec_dir.display().to_string(),
        recording_dir_writable,
        disk_free_gib,
        plugin_dir: plugin_dir.display().to_string(),
        plugins,
    };
    tpl.render()
        .map(Html)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(unix)]
fn disk_free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    if rc != 0 {
        return None;
    }
    Some(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(not(unix))]
fn disk_free_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

pub fn router() -> Router<AppState> {
    Router::new().route("/system/status", get(status))
}
