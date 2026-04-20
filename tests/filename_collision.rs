//! Two simultaneous streams resolving to the same template path must
//! produce two distinct files. Regression guard for the silent-overwrite
//! bug the prior audit called out (P0).
#![allow(clippy::field_reassign_with_default)]

use std::fs;
use strivo_core::config::AppConfig;
use strivo_core::platform::PlatformKind;
use strivo_core::recording::build_output_path;
use tempfile::TempDir;

fn config_with_fixed_template(dir: &std::path::Path) -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.recording_dir = dir.to_path_buf();
    // Force a deterministic collision by dropping date/title/platform tokens.
    cfg.recording.filename_template = "{channel}.mkv".to_string();
    cfg
}

#[test]
fn build_output_path_disambiguates_existing_file() {
    let tmp = TempDir::new().unwrap();
    let cfg = config_with_fixed_template(tmp.path());

    // Pre-seed the "first" file exactly where the template would land.
    let first = cfg.recording_dir.join("acme.mkv");
    fs::create_dir_all(&cfg.recording_dir).unwrap();
    fs::write(&first, b"existing").unwrap();

    let resolved = build_output_path(&cfg, "acme", PlatformKind::Twitch, Some("title"));
    assert_ne!(resolved, first, "must not collide with an existing file");
    assert!(!resolved.exists(), "returned path must still be free");
    assert_eq!(resolved.extension().and_then(|s| s.to_str()), Some("mkv"));
    let stem = resolved.file_stem().unwrap().to_string_lossy().to_string();
    assert!(
        stem.starts_with("acme_"),
        "disambiguated stem must keep the original prefix: got {stem:?}"
    );
}

#[test]
fn build_output_path_chain_of_collisions() {
    let tmp = TempDir::new().unwrap();
    let cfg = config_with_fixed_template(tmp.path());
    fs::create_dir_all(&cfg.recording_dir).unwrap();
    fs::write(cfg.recording_dir.join("acme.mkv"), b"").unwrap();
    fs::write(cfg.recording_dir.join("acme_1.mkv"), b"").unwrap();
    fs::write(cfg.recording_dir.join("acme_2.mkv"), b"").unwrap();

    let resolved = build_output_path(&cfg, "acme", PlatformKind::Twitch, Some("t"));
    assert!(!resolved.exists());
    assert_eq!(
        resolved,
        cfg.recording_dir.join("acme_3.mkv"),
        "disambiguator must pick the next free counter"
    );
}

#[test]
fn build_output_path_preserves_unique_name() {
    let tmp = TempDir::new().unwrap();
    let cfg = config_with_fixed_template(tmp.path());
    fs::create_dir_all(&cfg.recording_dir).unwrap();

    let resolved = build_output_path(&cfg, "solo", PlatformKind::YouTube, Some("title"));
    assert_eq!(resolved, cfg.recording_dir.join("solo.mkv"));
}
