pub mod park;
pub mod task;
pub mod types;
pub mod worker;

use crate::{version::Version, version_specialized};
use core::str;
use log::info;
use std::fmt::Display;

use super::{AsyncError, Future, TaskBacktrace};

version_specialized!(TokioVersion, "Tokio SemVer version");

impl Default for TokioVersion {
    fn default() -> Self {
        // the first supported version is default
        TokioVersion(Version((1, 40, 0)))
    }
}

impl Display for TokioVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("v{}.{}.x", self.0.0.0, self.0.0.1))
    }
}

/// Temporary function, parse tokio version from static string found in `rodata`` section.
///
/// WAITFORFIX: https://github.com/tokio-rs/tokio/issues/6950
pub fn extract_tokio_version_naive(rodata: &[u8]) -> Option<TokioVersion> {
    const TOKIO_V_TPL: &str = "tokio-1.";

    let tpl = TOKIO_V_TPL.as_bytes();
    let pos = rodata.windows(tpl.len()).position(|w| w == tpl)?;
    // get next number between dots
    let mut pos = pos + tpl.len();
    let mut minor = vec![];
    while rodata[pos] != b'.' || pos >= rodata.len() {
        minor.push(rodata[pos]);
        pos += 1;
    }
    let minor = str::from_utf8(&minor).ok()?;
    let version = TokioVersion(Version((1, minor.parse().ok()?, 0)));
    info!(target: "debugger", "tokio runtime {version} discovered");

    Some(version)
}
