use crate::oracle::builtin::tokio::TokioOracle;
use crate::oracle::Oracle;

pub mod tokio;

/// Create an oracle specified by name.
///
/// # Arguments
///
/// * `name`: oracle name
pub fn make_builtin(name: &str) -> Option<impl Oracle> {
    match name {
        "tokio" => Some(TokioOracle::new()),
        _ => None,
    }
}
