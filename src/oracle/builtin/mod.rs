use crate::oracle::Oracle;
use crate::oracle::builtin::nop::NopOracle;
use crate::oracle::builtin::tokio::TokioOracle;
use std::sync::Arc;

pub mod nop;
pub mod tokio;

/// Create an oracle specified by name.
///
/// # Arguments
///
/// * `name`: oracle name
pub fn make_builtin(name: &str) -> Option<Arc<dyn Oracle>> {
    match name {
        "tokio" => Some(Arc::new(TokioOracle::new())),
        "nop" => Some(Arc::new(NopOracle::default())),
        _ => None,
    }
}
