use proc_maps::MapRange;

/// Types can implement this trait for include cache functionality.
pub trait TryGetOrInsert<T> {
    /// Returns inner value if exists, otherwise execute function `f`, then save returned value and return it.
    ///
    /// # Arguments
    ///
    /// * `f`: function executed if inner value not exists.
    fn try_get_or_insert_with<E, F>(&mut self, f: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>;
}

impl<T> TryGetOrInsert<T> for Option<T> {
    fn try_get_or_insert_with<E, F>(&mut self, f: F) -> Result<&T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        match self {
            Some(value) => Ok(value),
            None => Ok(self.insert(f()?)),
        }
    }
}

pub trait PopIf<T> {
    fn pop_if_cond<F>(&mut self, pred: F) -> Option<T>
    where
        F: FnOnce(&Self) -> bool;

    fn pop_if_single_el(&mut self) -> Option<T>;
}

impl<T> PopIf<T> for Vec<T> {
    fn pop_if_cond<F>(&mut self, pred: F) -> Option<T>
    where
        F: FnOnce(&Self) -> bool,
    {
        if pred(self) { self.pop() } else { None }
    }

    fn pop_if_single_el(&mut self) -> Option<T> {
        self.pop_if_cond(|v| v.len() == 1)
    }
}

/// Return true if address exist in VAS.
pub fn region_exist(pid: nix::unistd::Pid, addr: u64) -> std::io::Result<bool> {
    let proc_maps: Vec<MapRange> = proc_maps::get_process_maps(pid.as_raw())?;
    Ok(proc_maps.iter().any(|range| range.start() == addr as usize))
}

/// Return true if address not exist in VAS.
pub fn region_non_exist(pid: nix::unistd::Pid, addr: u64) -> std::io::Result<bool> {
    region_exist(pid, addr).map(|exist| !exist)
}
