#[macro_export]
macro_rules! weak_error {
    ($res: expr) => {
        match $res {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!(target: "debugger", "{}", e);
                None
            }
        }
    };
}
