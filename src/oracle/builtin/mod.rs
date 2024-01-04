use crate::oracle::builtin::tokio::TokioOracle;
use crate::oracle::Oracle;

pub mod tokio;

/// Create an oracle specified by name.
///
/// # Arguments
///
/// * `name`: oracle name
pub fn create_builtin(name: &str) -> Option<Box<dyn Oracle>> {
    match name {
        "tokio" => Some(Box::new(TokioOracle::new())),
        _ => None,
    }
}
