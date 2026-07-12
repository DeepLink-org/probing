//! Federated query policy helpers (mirrors `probing_core::core::federation`).

use std::sync::LazyLock;

const FANOUT_STRICT_ENV: &str = "PROBING_FANOUT_STRICT";

static FANOUT_STRICT: LazyLock<bool> = LazyLock::new(|| {
    std::env::var(FANOUT_STRICT_ENV)
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
});

pub fn fanout_strict_enabled() -> bool {
    *FANOUT_STRICT
}
