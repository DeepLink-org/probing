use std::sync::{LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use probing_core::sync::{read_rwlock, write_rwlock};

pub static PROBING_ADDRESS: LazyLock<RwLock<String>> =
    LazyLock::new(|| RwLock::new(Default::default()));

pub fn read_probing_address() -> RwLockReadGuard<'static, String> {
    read_rwlock(&PROBING_ADDRESS, "PROBING_ADDRESS")
}

pub fn write_probing_address() -> RwLockWriteGuard<'static, String> {
    write_rwlock(&PROBING_ADDRESS, "PROBING_ADDRESS")
}
