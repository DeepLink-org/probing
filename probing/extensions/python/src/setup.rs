pub fn register_signal_handler<F>(sig: std::ffi::c_int, handler: F)
where
    F: Fn() + Sync + Send + 'static,
{
    unsafe {
        match signal_hook_registry::register_unchecked(sig, move |_: &_| handler()) {
            Ok(_) => {
                log::debug!("Registered signal handler for signal {sig}");
            }
            Err(e) => log::error!("Failed to register signal handler: {e}"),
        }
    };
}

#[ctor]
fn setup() {
    use crate::python::{set_enabled, should_enable_probing};

    probing_core::install_panic_hook();

    // Auto-print the crashing thread's backtrace on fatal signals. Opt out with
    // `PROBING_CRASH_BACKTRACE=0` if it interferes with the host app.
    if std::env::var("PROBING_CRASH_BACKTRACE").as_deref() != Ok("0") {
        crate::features::crash::install_crash_handler();
    }

    if should_enable_probing() {
        set_enabled(true);
    }

    #[cfg(unix)]
    if cfg!(test) {
        // Unit-test processes must not run the SIGUSR2 stack handler: it calls
        // backtrace/Python from signal context and aborts on stray delivery.
        unsafe {
            nix::libc::signal(nix::libc::SIGUSR2, nix::libc::SIG_IGN);
        }
    } else {
        register_signal_handler(
            nix::libc::SIGUSR2,
            crate::features::stack_tracer::backtrace_signal_handler,
        );
    }
}
