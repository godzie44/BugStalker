use crate::weak_error;
use itertools::Itertools;
use object::{Object, ObjectSection};
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

/// Supported rustc version diapasons.
static SUPPORTED_RUSTC: &[(Version, Version)] = &[
    (Version((1, 75, 0)), Version((1, 75, u32::MAX))),
    (Version((1, 76, 0)), Version((1, 76, u32::MAX))),
    (Version((1, 77, 0)), Version((1, 77, u32::MAX))),
    (Version((1, 78, 0)), Version((1, 78, u32::MAX))),
    (Version((1, 79, 0)), Version((1, 79, u32::MAX))),
    (Version((1, 80, 0)), Version((1, 80, u32::MAX))),
    (Version((1, 81, 0)), Version((1, 81, u32::MAX))),
    (Version((1, 82, 0)), Version((1, 82, u32::MAX))),
    (Version((1, 83, 0)), Version((1, 83, u32::MAX))),
    (Version((1, 84, 0)), Version((1, 84, u32::MAX))),
    (Version((1, 85, 0)), Version((1, 85, u32::MAX))),
    (Version((1, 86, 0)), Version((1, 86, u32::MAX))),
];

pub fn supported_versions_to_string() -> String {
    format!(
        "[{}]",
        SUPPORTED_RUSTC
            .iter()
            .map(|(v, _)| format!("{}.{}.x", v.0.0, v.0.1))
            .join(", ")
    )
}

/// Check a rustc version, return true if a version supported, false otherwise. False positive.
pub fn probe_file(obj: &object::File) -> bool {
    let Some(comment_sect) = obj.section_by_name(".comment") else {
        return true;
    };
    let Ok(data) = comment_sect.data() else {
        return true;
    };
    let Ok(string_data) = std::str::from_utf8(data) else {
        return true;
    };

    if let Some(version) = Version::rustc_parse(string_data) {
        return SUPPORTED_RUSTC
            .iter()
            .any(|(v_min, v_max)| version >= *v_min && version <= *v_max);
    }

    true
}
