//! Lazy `log` init for the HCCL shim (dlopen'd without the main probing module).

use std::sync::Once;

static INIT: Once = Once::new();

pub fn ensure() {
    INIT.call_once(|| {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::new()
                .default_filter_or("warn")
                .filter("PROBING_LOGLEVEL"),
        )
        .format_target(true)
        .try_init();
    });
}

pub fn info(msg: impl AsRef<str>) {
    ensure();
    log::info!(target: "probing-hccl-shim", "{}", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    ensure();
    log::warn!(target: "probing-hccl-shim", "{}", msg.as_ref());
}
