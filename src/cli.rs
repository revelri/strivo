use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "streavo", version, about = "TUI Live Stream PVR")]
pub struct Args {
    /// Path to config file
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info", global = true)]
    pub log_level: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Manage log file
    Log {
        #[command(subcommand)]
        action: LogAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// List all configuration values
    List,
    /// Print the config file path
    Path,
    /// Get a specific config value
    Get {
        /// Config key (recording_dir, poll_interval, transcode, filename_template,
        /// twitch.client_id, youtube.client_id, youtube.client_secret, youtube.cookies_path)
        key: String,
    },
    /// Set a config value and save
    Set {
        /// Config key
        key: String,
        /// New value
        value: String,
    },
    /// Reset config to defaults (preserves platform credentials)
    Reset,
}

#[derive(Subcommand, Debug)]
pub enum LogAction {
    /// Print the log file path
    Path,
    /// Clear the log file
    Clear,
    /// Tail the log file (live, Ctrl-C to stop)
    Tail {
        /// Number of lines to show initially
        #[arg(short, long, default_value = "50")]
        lines: usize,
    },
}
