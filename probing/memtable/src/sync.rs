use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock_mutex<'a, T>(m: &'a Mutex<T>, label: &str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poison) => {
            log::warn!("{label} mutex poisoned; recovering");
            poison.into_inner()
        }
    }
}
