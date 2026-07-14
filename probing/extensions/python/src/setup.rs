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
        // Unit-test processes must not run the SIGUSR2 stack handler: it captures
        // stacks from signal context and aborts on stray delivery.
        unsafe {
            nix::libc::signal(nix::libc::SIGUSR2, nix::libc::SIG_IGN);
        }
    } else {
        crate::features::stack_capture::install_sigusr2_handler();
    }
}
