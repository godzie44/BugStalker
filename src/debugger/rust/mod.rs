use crate::debugger::error::Error;
use crate::debugger::error::Error::{DefaultToolchainNotFound, UnrecognizedRustupOut};
use log::warn;
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::process::Command;

static ENVIRONMENT: OnceCell<Environment> = OnceCell::new();

#[derive(Debug)]
pub struct Environment {
    pub toolchain: Option<Toolchain>,
    pub std_lib_path: Option<PathBuf>,
}

impl Environment {
    pub fn current() -> &'static Self {
        ENVIRONMENT.get().unwrap()
    }

    pub fn init(std_lib_path: Option<PathBuf>) {
        let toolchain = default_toolchain();
        if let Err(ref e) = toolchain {
            warn!(target: "debugger", "detect toolchain: {e}")
        }
        if ENVIRONMENT
            .set(Environment {
                std_lib_path: std_lib_path
                    .or_else(|| toolchain.as_ref().ok().map(|t| t.std_lib_path())),
                toolchain: toolchain.ok(),
            })
            .is_err()
        {
            warn!(target: "debugger", "rust env already set")
        }
    }
}

#[derive(Debug)]
pub struct Toolchain {
    #[allow(unused)]
    name: String,
    path: PathBuf,
}

impl Toolchain {
    pub fn std_lib_path(&self) -> PathBuf {
        self.path.clone().join("lib/rustlib/src/rust")
    }
}

pub fn default_toolchain() -> Result<Toolchain, Error> {
    let rustup_out = Command::new("rustup")
        .args(["toolchain", "list", "-v"])
        .output()?;
    let toolchains = String::from_utf8(rustup_out.stdout)?;
    let toolchain = toolchains
        .lines()
        .find(|line| line.contains("default"))
        .ok_or(DefaultToolchainNotFound)?;

    let toolchain_verbose_parts = toolchain.split_whitespace().collect::<Vec<_>>();

    if toolchain_verbose_parts.len() < 3 {
        return Err(UnrecognizedRustupOut);
    }

    Ok(Toolchain {
        name: toolchain_verbose_parts.first().unwrap().to_string(),
        path: PathBuf::from(toolchain_verbose_parts.last().unwrap()),
    })
}
