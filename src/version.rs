use crate::weak_error;
use once_cell::sync;
use regex::Regex;

/// Compiler SemVer version.
#[derive(PartialEq, PartialOrd)]
pub struct Version(pub (u32, u32, u32));

impl Version {
    /// Parse rustc version from strings like:
    /// "GCC: (Ubuntu 11.4.0-1ubuntu1~22.04) 11.4.0.rustc version 1.75.0 (82e1608df 2023-12-21)."
    pub fn rustc_parse(s: &str) -> Option<Self> {
        static V_RE: sync::Lazy<Regex> = sync::Lazy::new(|| {
            Regex::new(r"rustc version (\d+)\.(\d+)\.(\d+)").expect("must compile")
        });

        if let Some((_, [major, minor, patch])) = V_RE.captures_iter(s).next().map(|c| c.extract())
        {
            let major = weak_error!(major.parse::<u32>())?;
            let minor = weak_error!(minor.parse::<u32>())?;
            let patch = weak_error!(patch.parse::<u32>())?;
            return Some(Version((major, minor, patch)));
        }
        None
    }
}

impl Default for Version {
    fn default() -> Self {
        // the first supported version is default
        Version((1, 75, 0))
    }
}

/// Execute expression depending on compiler version.
#[macro_export]
macro_rules! version_switch {
            ($lang_v:expr, $($v1:tt ..= $v2:expr => $code: expr),+ $(,)?) => {
                $(
                    if $lang_v >= $crate::version::Version($v1) && $lang_v <= $crate::version::Version($v2) {
                        Some($code)
                    } else
                )*
                {
                    None
                }
            };
        }
