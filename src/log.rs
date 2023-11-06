use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(true);

#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn disable() {
    ENABLED.store(false, Ordering::SeqCst)
}

pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst)
}

#[macro_export]
macro_rules! bs_info {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::info!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::info!($($arg)+)
        }
    };
}

#[macro_export]
macro_rules! bs_warn {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::warn!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::warn!($($arg)+)
        }
    };
}

#[macro_export]
macro_rules! bs_error {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::error!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::error!($($arg)+)
        }
    };
}

#[macro_export]
macro_rules! bs_debug {
    (target: $target:expr, $($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::debug!(target: $target, $($arg)+)
        }
    };
    ($($arg:tt)+) => {
        if $crate::log::is_enabled() {
            log::debug!($($arg)+)
        }
    };
}
