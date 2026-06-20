use anyhow::{Context, Result};
use std::path::Path;

/// Common trait for platform-specific injection implementations
pub trait InjectionTrait {
    /// Inject the shellcode into the given tracee
    fn inject(
        proc: &crate::inject::Process,
        tracer: &mut pete::Ptracer,
        tracee: pete::Tracee,
    ) -> Result<Self>
    where
        Self: Sized;

    /// Execute the injection to load the library
    fn execute(&mut self, filename: &Path) -> Result<()>;

    /// Set environment variables
    fn setenv(&mut self, name: Option<&str>, value: Option<&str>) -> Result<()>;

    /// Remove the injection and restore original state
    fn remove(self) -> Result<()>;
}

/// Generic injection function that works with any platform
pub fn perform_injection<T>(
    proc: &crate::inject::Process,
    tracer: &mut pete::Ptracer,
    tracee: pete::Tracee,
    library: &Path,
    settings: Vec<String>,
) -> Result<()>
where
    T: InjectionTrait,
{
    let mut injection = T::inject(proc, tracer, tracee)?;

    // Process environment settings
    for setting in settings {
        if let Some((name, value)) = setting.split_once('=') {
            let name = name.to_uppercase();
            let value = value.to_string();

            injection
                .setenv(Some(&name), Some(&value))
                .context("failed to prepare env string")?;
        }
    }

    // Execute the injection
    injection
        .execute(library)
        .context("failed to execute shellcode")?;

    // Clean up
    injection.remove().context("failed to remove shellcode")?;

    Ok(())
}
