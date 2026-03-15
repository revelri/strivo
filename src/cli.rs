use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "streavo", version, about = "TUI Live Stream PVR")]
pub struct Args {
    /// Path to config file
    #[arg(short, long)]
    pub config: Option<std::path::PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
}
