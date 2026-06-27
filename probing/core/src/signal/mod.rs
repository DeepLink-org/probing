//! Cross-platform signal helpers shared by collectors and runtime features.

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::send_sigusr2_to_thread_id;
