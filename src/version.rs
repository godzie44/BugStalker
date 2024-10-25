use crate::weak_error;
use itertools::Itertools;
use object::{Object, ObjectSection};
use once_cell::sync;
use regex::Regex;

/// Compiler SemVer version.
#[derive(PartialEq, PartialOrd, Debug)]
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

#[macro_export]
macro_rules! _version_switch {
    ($lang_v: expr, ($major_1: expr, $minor_1: expr) ..) => {
        $lang_v >= $crate::version::Version(($major_1, $minor_1, 0))
            && $lang_v <= $crate::version::Version(($major_1, u32::MAX, u32::MAX))
    };
    ($lang_v: expr, .. ($major_2: expr, $minor_2: expr)) => {
        $lang_v >= $crate::version::Version((1, 0, 0))
            && $lang_v < $crate::version::Version(($major_2, $minor_2, 0))
    };
    ($lang_v: expr, ($major_1: expr, $minor_1: expr) .. ($major_2: expr, $minor_2: expr)) => {
        $lang_v >= $crate::version::Version(($major_1, $minor_1, 0))
            && $lang_v < $crate::version::Version(($major_2, $minor_2, 0))
    };
}

/// Execute expression depending on compiler version.
#[macro_export]
macro_rules! version_switch {
    ($lang_v: expr, $($(($major_1: tt.$minor_1: tt))? .. $(($major_2: tt.$minor_2: tt))? => $code: expr),+ $(,)?) => {
        {
            $(
                 if $crate::_version_switch!($lang_v, $(($major_1, $minor_1))? .. $(($major_2, $minor_2))?) {
                     Some($code)
                 } else
            )*
            {
                None
            }

        }
    };
}

macro_rules! supported {
    ($($ver_major: tt . $ver_minor: expr);+ $(;)?) => {
        &[
            $(
                (Version(($ver_major, $ver_minor, 0)), Version(($ver_major, $ver_minor, u32::MAX))),
            )*
        ]
    };
}

/// Supported rustc version diapasons.
static SUPPORTED_RUSTC: &[(Version, Version)] = supported!(
    1 . 75;
    1 . 76;
    1 . 77;
    1 . 78;
    1 . 79;
    1 . 80;
    1 . 81;
    1 . 82;
);

pub fn supported_versions_to_string() -> String {
    format!(
        "[{}]",
        SUPPORTED_RUSTC
            .iter()
            .map(|(v, _)| format!("{}.{}.x", v.0 .0, v.0 .1))
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
