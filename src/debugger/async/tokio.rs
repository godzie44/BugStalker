use core::str;

use crate::{version::Version, version_specialized};

version_specialized!(TokioVersion, "Tokio SemVer version");

impl Default for TokioVersion {
    fn default() -> Self {
        // the first supported version is default
        TokioVersion(Version((1, 40, 0)))
    }
}

/// Temporary function, parse tokio version from static string found in `rodata`` section.
///
/// WAITFORFIX: https://github.com/tokio-rs/tokio/issues/6950
pub fn extract_version_naive(rodata: &[u8]) -> Option<TokioVersion> {
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
    Some(TokioVersion(Version((1, minor.parse().ok()?, 0))))
}
