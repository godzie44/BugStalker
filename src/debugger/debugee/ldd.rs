use std::path::{Path, PathBuf};

/// Parse output of `ldd` for finding shared object dependencies.
///
/// # Arguments
///
/// * `file`: path to program
pub fn find_dependencies(file: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut cmd = std::process::Command::new("ldd");
    let output = cmd.arg(file.to_string_lossy().as_ref()).output()?;
    let output = std::str::from_utf8(&output.stdout)?;
    Ok(parse_ldd_output(output))
}

fn parse_ldd_output(output: &str) -> Vec<PathBuf> {
    output
        .lines()
        .filter_map(|s| {
            let s = s.trim();
            let s = s.split("=>").last();
            s.and_then(|s| s.split_whitespace().next())
        })
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod test {
    use crate::debugger::debugee::ldd::parse_ldd_output;
    use std::path::PathBuf;

    #[test]
    fn test_ldd_output_parsing() {
        struct TestCase {
            ldd_output: &'static str,
            expected: Vec<PathBuf>,
        }
        let test_cases = [
            TestCase {
                ldd_output: "	linux-vdso.so.1 (0x00007ffd69ff4000)
	libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f66730e7000)
	libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f6672e00000)
	/lib64/ld-linux-x86-64.so.2 (0x00007f66732b3000)
",
                expected: vec![
                    PathBuf::from("linux-vdso.so.1"),
                    PathBuf::from("/lib/x86_64-linux-gnu/libgcc_s.so.1"),
                    PathBuf::from("/lib/x86_64-linux-gnu/libc.so.6"),
                    PathBuf::from("/lib64/ld-linux-x86-64.so.2"),
                ],
            },
            TestCase {
                ldd_output: "/lib/x86_64-linux-gnu/libc.so.6
	/lib64/ld-linux-x86-64.so.2
",
                expected: vec![
                    PathBuf::from("/lib/x86_64-linux-gnu/libc.so.6"),
                    PathBuf::from("/lib64/ld-linux-x86-64.so.2"),
                ],
            },
        ];

        for tc in test_cases {
            let result = parse_ldd_output(tc.ldd_output);
            assert_eq!(result, tc.expected);
        }
    }
}
