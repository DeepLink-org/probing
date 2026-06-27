use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub fn lock_mutex<'a, T>(m: &'a Mutex<T>, label: &str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(guard) => guard,
        Err(e) => {
            log::warn!("{label} mutex poisoned; recovering");
            e.into_inner()
        }
    }
}

pub fn read_rwlock<'a, T>(rw: &'a RwLock<T>, label: &str) -> RwLockReadGuard<'a, T> {
    match rw.read() {
        Ok(guard) => guard,
        Err(e) => {
            log::warn!("{label} RwLock poisoned; recovering");
            e.into_inner()
        }
    }
}

pub fn write_rwlock<'a, T>(rw: &'a RwLock<T>, label: &str) -> RwLockWriteGuard<'a, T> {
    match rw.write() {
        Ok(guard) => guard,
        Err(e) => {
            log::warn!("{label} RwLock poisoned; recovering");
            e.into_inner()
        }
    }
}
