use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Address to listen on (default: 127.0.0.1:4711)
    #[clap(long, default_value = "127.0.0.1:4711")]
    pub listen: String,

    /// Exit after the first debug session ends (single-client mode).
    #[clap(long)]
    pub oneshot: bool,

    /// Optional log file for adapter diagnostics (no output to stdout).
    #[clap(long)]
    pub log_file: Option<std::path::PathBuf>,

    /// Trace DAP traffic (requests/responses/events) into the log file.
    /// Requires --log-file.
    #[clap(long)]
    pub trace_dap: bool,

    /// Discover a specific oracle (maybe more than one)
    #[clap(short, long)]
    pub oracle: Vec<String>,
}
