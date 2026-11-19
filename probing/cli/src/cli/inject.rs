use std::path::PathBuf;

use crate::inject::{Injector, Process};
use anyhow::{anyhow, Error, Result};
use clap::Args;
use probing_proto::prelude::Query;

use super::ctrl;
use super::ctrl::ProbeEndpoint;

/// Inject into the target process
#[derive(Args, Default, Debug)]
pub struct InjectCommand {
    #[arg(short = 'D', long = "define", num_args = 1..)]
    settings: Vec<String>,
}

impl InjectCommand {
    /// Locate the native library that starts the probing runtime after `dlopen`.
    ///
    /// The Python wheel installs the runtime as `probing/_core*.so`, while the
    /// Rust binary distribution keeps `libprobing.so` next to the executable.
    /// `PROBING_INJECT_LIB` lets the Python CLI wrapper point at the wheel
    /// extension explicitly.
    fn resolve_injection_library(&self) -> Result<PathBuf> {
        if let Ok(path) = std::env::var("PROBING_INJECT_LIB") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Ok(path);
            }
            return Err(anyhow!(
                "PROBING_INJECT_LIB does not point to a file: {}",
                path.display()
            ));
        }

        let sibling = std::fs::read_link("/proc/self/exe")?.with_file_name("libprobing.so");
        if sibling.is_file() {
            return Ok(sibling);
        }

        Err(anyhow!(
            "cannot locate the probing injection library; \
             set PROBING_INJECT_LIB to probing/_core*.so (wheel) or place libprobing.so next to the executable"
        ))
    }

    fn check_library(&self, pid: i32, _lib_hint: &str) -> Result<bool> {
        Ok(procfs::process::Process::new(pid)?.maps()?.iter().any(|m| {
            matches!(
                &m.pathname,
                procfs::process::MMapPath::Path(p)
                    if p.file_name()
                        .map(|n| {
                            let name = n.to_string_lossy();
                            name.contains("libprobing")
                                || (name.starts_with("_core") && name.ends_with(".so"))
                        })
                        .unwrap_or(false)
            )
        }))
    }

    fn build_settings(&self) -> Vec<String> {
        self.settings
            .iter()
            .map(|setting| {
                if setting.starts_with("probing.") {
                    setting.replace(".", "_")
                } else {
                    setting.clone()
                }
            })
            .collect()
    }

    fn inject(&self, pid: i32) -> Result<()> {
        let soname = self.resolve_injection_library()?;
        let settings = self.build_settings();

        println!("Injecting {} into {}", soname.display(), pid);
        Injector::attach(Process::get(pid as u32).map_err(Error::msg)?)
            .map_err(Error::msg)?
            .inject(&soname, settings)
            .map_err(|e| anyhow!("Failed to inject probing: {}\n\t{}", e, e.root_cause()))
    }

    pub async fn run(&self, ctrl: ProbeEndpoint) -> Result<()> {
        match ctrl {
            ProbeEndpoint::Ptrace { pid } | ProbeEndpoint::Local { pid } => {
                if !self.check_library(pid, "libprobing.so").unwrap_or(false) {
                    // Do not gate injection on the target's maps: some
                    // container/PID-namespace setups hide the interpreter
                    // path from procfs while ptrace still works.
                    self.inject(pid)
                } else {
                    let settings = self.build_settings();
                    let query: Vec<String> = settings
                        .iter()
                        .map(|setting| format!("set {setting}"))
                        .collect();
                    let query = query.join(";");

                    ctrl::query(
                        ctrl,
                        Query {
                            expr: query,
                            opts: None,
                        },
                    )
                    .await
                }
            }
            _ => Ok(()),
        }
    }
}
