//! Machine identifier — stable across reboots, unique per install.
//!
//! Source order (first hit wins):
//!   1. Linux: `/etc/machine-id` (systemd; written at install)
//!   2. macOS: `IOPlatformUUID` via `ioreg`
//!   3. Windows: `MachineGuid` in `HKLM\SOFTWARE\Microsoft\Cryptography`
//!   4. Anywhere: a v4 UUID we generate once and persist under the
//!      state dir so subsequent reads are stable.
//!
//! The OS-sourced IDs are NOT user-secret (any installed app can read
//! them), and we only ever hash them before sending — see
//! `hashed_machine_id()`. The activation server stores the hash, not
//! the raw value.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;

static CACHED: OnceLock<String> = OnceLock::new();

/// Returns the stable machine identifier, resolving + caching on first
/// call. Never panics — falls through to the persisted-UUID path on any
/// failure of the OS-specific reader.
pub fn machine_id() -> String {
    CACHED
        .get_or_init(|| resolve().unwrap_or_else(|_| persisted_fallback()))
        .clone()
}

/// SHA-256(machine_id) hex-encoded — what we actually send to the
/// activation backend. Lets us correlate seats without storing the raw
/// platform identifier server-side.
pub fn hashed_machine_id() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(machine_id().as_bytes());
    hex::encode(h.finalize())
}

fn resolve() -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/etc/machine-id") {
            let t = s.trim();
            if !t.is_empty() {
                return Ok(t.to_string());
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // ioreg -rd1 -c IOPlatformExpertDevice | awk '/IOPlatformUUID/ ...'
        let out = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if let Some(idx) = line.find("IOPlatformUUID") {
                if let Some(eq) = line[idx..].find('=') {
                    let val = line[idx + eq + 1..].trim();
                    let uuid = val.trim_matches(&['"', ' '] as &[_]);
                    if !uuid.is_empty() {
                        return Ok(uuid.to_string());
                    }
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        // reg query HKLM\SOFTWARE\Microsoft\Cryptography /v MachineGuid
        let out = std::process::Command::new("reg")
            .args([
                "query",
                "HKLM\\SOFTWARE\\Microsoft\\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            let lower = line.to_lowercase();
            if lower.contains("machineguid") {
                // Format: "    MachineGuid    REG_SZ    <uuid>"
                if let Some(guid) = line.split_whitespace().last() {
                    if !guid.is_empty() {
                        return Ok(guid.to_string());
                    }
                }
            }
        }
    }
    anyhow::bail!("no OS-provided machine id")
}

/// `~/.local/share/strivo/machine_id` — generated once, then re-read on
/// subsequent calls. Lets containers and stripped-down OSes still get a
/// stable per-install identity.
fn persisted_fallback() -> String {
    let path = fallback_path();
    if let Ok(s) = std::fs::read_to_string(&path) {
        let t = s.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &id);
    id
}

fn fallback_path() -> PathBuf {
    crate::config::AppConfig::state_dir().join("machine_id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_id_is_stable_across_calls() {
        let a = machine_id();
        let b = machine_id();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn hash_is_deterministic_and_hex() {
        let h1 = hashed_machine_id();
        let h2 = hashed_machine_id();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // sha256 hex
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
