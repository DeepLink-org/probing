use std::backtrace::Backtrace;
use std::panic;
use std::sync::Once;

static INSTALL_PANIC_HOOK: Once = Once::new();

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "full" | "on")
        }
        Err(_) => false,
    }
}

fn backtrace_enabled() -> bool {
    cfg!(debug_assertions) || env_flag("RUST_BACKTRACE") || env_flag("PROBING_RUST_BACKTRACE")
}

/// Install a panic hook that prints thread context and (when enabled) a Rust backtrace.
///
/// Enable backtraces with `RUST_BACKTRACE=1` or `PROBING_RUST_BACKTRACE=1`.
/// Debug builds always capture a backtrace.
pub fn install_panic_hook() {
    INSTALL_PANIC_HOOK.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("<unnamed>");
            let thread_id = format!("{:?}", thread.id());

            eprintln!();
            eprintln!("=== probing panic (thread={thread_name}, id={thread_id}) ===");
            if let Some(location) = info.location() {
                eprintln!(
                    "at {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }
            if let Some(payload) = info.payload().downcast_ref::<&str>() {
                eprintln!("message: {payload}");
            } else if let Some(payload) = info.payload().downcast_ref::<String>() {
                eprintln!("message: {payload}");
            }

            if backtrace_enabled() {
                let backtrace = Backtrace::force_capture();
                eprintln!("rust backtrace:\n{backtrace}");
            } else {
                eprintln!(
                    "hint: set RUST_BACKTRACE=1 or PROBING_RUST_BACKTRACE=1 for a rust backtrace"
                );
            }
            eprintln!("=== end probing panic ===");
            eprintln!();

            default_hook(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_hook_installs_once() {
        install_panic_hook();
        install_panic_hook();
    }
}
