use std::collections::HashMap;
use std::fmt::Display;

use async_trait::async_trait;

use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;
use probing_core::core::Result as EngineResult;
use probing_core::run_on_native_thread;
use probing_proto::prelude::CallFrame;
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyString};
use pyo3::Python;

pub use exttbls::PyExternalTableConfig;
pub use exttbls::{register_table_docs, ExternalTable};
pub use tbls::PythonProbeDataSource;

use crate::features::stack_tracer::{SignalTracer, StackTracer};
use crate::python::enable_crash_handler;
use crate::python::enable_monitoring;

mod exttbls;
mod profile_sql;
mod tbls;

pub use tbls::PythonNamespace;

/// Collection of Python extensions loaded into the system
#[derive(Debug, Default)]
struct PyExtList(HashMap<String, pyo3::Py<pyo3::PyAny>>);

impl Display for PyExtList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for ext in self.0.keys() {
            if first {
                write!(f, "{ext}")?;
                first = false;
            } else {
                write!(f, ", {ext}")?;
            }
        }
        Ok(())
    }
}

/// Python integration with the probing system
#[derive(Debug, Default, ProbeExtension)]
pub struct PythonExt {
    /// Enable the crash handler module (`probing.crash`). Aliases: `crash.handler`.
    #[option(aliases = ["crash.handler", "crash.enabled"])]
    crash_handler: Maybe<String>,

    /// Path to Python monitoring handler script
    #[option()]
    monitoring: Maybe<String>,

    /// Enable Python extensions by setting `python.enabled=<extension_statement>`
    #[option()]
    enabled: PyExtList,

    /// Disable Python extension by setting `python.disabled=<extension_statement>`
    #[option()]
    disabled: Maybe<String>,
}

#[async_trait]
impl ProbeExtensionCall for PythonExt {
    async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> EngineResult<Vec<u8>> {
        log::debug!(
            "Python extension call - path: {}, params: {:?}, body_size: {}",
            path,
            params,
            body.len()
        );

        let normalized_path = path.trim_start_matches('/');
        if normalized_path.starts_with("crash/") {
            return crate::features::crash::handle_http(normalized_path, params, body)
                .map_err(EngineError::plugin);
        }
        call_python_handler(normalized_path, params, body).await
    }
}

impl PythonExt {
    /// Set up a Python crash handler
    fn set_crash_handler(&mut self, crash_handler: Maybe<String>) -> EngineResult<()> {
        match self.crash_handler {
            Maybe::Just(_) => Err(EngineError::ReadOnlyOption(
                Self::OPTION_CRASH_HANDLER.to_string(),
            )),
            Maybe::Nothing => match &crash_handler {
                Maybe::Nothing => Err(EngineError::InvalidOptionValue(
                    Self::OPTION_CRASH_HANDLER.to_string(),
                    crash_handler.clone().into(),
                )),
                Maybe::Just(handler) => {
                    self.crash_handler = crash_handler.clone();
                    let lowered = handler.trim().to_ascii_lowercase();
                    if lowered == "off" || lowered == "0" || lowered == "false" {
                        log::info!("Python crash handler disabled");
                        return Ok(());
                    }
                    match enable_crash_handler() {
                        Ok(_) => {
                            log::info!("Python crash handler enabled: {handler}");
                            Ok(())
                        }
                        Err(e) => {
                            log::error!("Failed to enable crash handler '{handler}': {e}");
                            Err(EngineError::InvalidOptionValue(
                                Self::OPTION_CRASH_HANDLER.to_string(),
                                handler.to_string(),
                            ))
                        }
                    }
                }
            },
        }
    }

    /// Set up Python monitoring
    fn set_monitoring(&mut self, monitoring: Maybe<String>) -> EngineResult<()> {
        log::debug!("Setting Python monitoring: {monitoring}");
        match self.monitoring {
            Maybe::Just(_) => Err(EngineError::ReadOnlyOption(
                Self::OPTION_MONITORING.to_string(),
            )),
            Maybe::Nothing => match &monitoring {
                Maybe::Nothing => Err(EngineError::InvalidOptionValue(
                    Self::OPTION_MONITORING.to_string(),
                    monitoring.clone().into(),
                )),
                Maybe::Just(handler) => {
                    self.monitoring = monitoring.clone();
                    match enable_monitoring(handler) {
                        Ok(_) => {
                            log::info!("Python monitoring enabled: {handler}");
                            Ok(())
                        }
                        Err(e) => {
                            log::error!("Failed to enable monitoring '{handler}': {e}");
                            Err(EngineError::InvalidOptionValue(
                                Self::OPTION_MONITORING.to_string(),
                                handler.to_string(),
                            ))
                        }
                    }
                }
            },
        }
    }

    /// Enable a Python extension from code string
    fn set_enabled(&mut self, enabled: Maybe<String>) -> EngineResult<()> {
        let ext = match &enabled {
            Maybe::Nothing => {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_ENABLED.to_string(),
                    enabled.clone().into(),
                ));
            }
            Maybe::Just(e) => e,
        };

        if self.enabled.0.contains_key(ext) {
            return Err(EngineError::plugin(format!(
                "Python extension '{ext}' is already enabled"
            )));
        }

        let pyext = execute_python_code(ext)
            .map_err(|e| EngineError::invalid_option(Self::OPTION_ENABLED, e))?;

        self.enabled.0.insert(ext.clone(), pyext);
        log::info!("Python extension enabled: {ext}");
        log::debug!("Current enabled extensions: {}", self.enabled);

        Ok(())
    }

    /// Disable a previously enabled Python extension
    fn set_disabled(&mut self, disabled: Maybe<String>) -> EngineResult<()> {
        let ext = match &disabled {
            Maybe::Nothing => {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_DISABLED.to_string(),
                    disabled.clone().into(),
                ));
            }
            Maybe::Just(e) => e,
        };

        if let Some(pyext) = self.enabled.0.remove(ext) {
            log::info!("Disabling Python extension: {ext}");
            let ext_name = ext.clone();

            run_on_native_thread(move || {
                Python::attach(|py| match pyext.call_method0(py, "deinit") {
                    Ok(_) => {
                        log::debug!("Extension '{ext_name}' deinitialized successfully");
                        Ok(())
                    }
                    Err(e) => {
                        log::error!("Failed to call deinit method on '{ext_name}': {e}");
                        Err(EngineError::plugin(format!(
                            "Failed to call deinit method on '{ext_name}': {e}"
                        )))
                    }
                })
            })
        } else {
            log::debug!("Python extension '{ext}' was not enabled, nothing to disable");
            Ok(())
        }
    }
}

/// Convert a PyO3 result into an [`EngineError`] with a description of the failed
/// step, so the Python boundary can use `.py_context("…")?` instead of an
/// inline `.map_err(|e| EngineError::plugin(format!("…: {e}")))` at every call site.
trait PyContext<T> {
    fn py_context(self, ctx: &str) -> EngineResult<T>;
    fn py_context_with(self, ctx: impl FnOnce() -> String) -> EngineResult<T>;
}

impl<T> PyContext<T> for PyResult<T> {
    fn py_context(self, ctx: &str) -> EngineResult<T> {
        self.map_err(|e| EngineError::plugin(format!("{ctx}: {e}")))
    }

    fn py_context_with(self, ctx: impl FnOnce() -> String) -> EngineResult<T> {
        self.map_err(|e| EngineError::plugin(format!("{}: {e}", ctx())))
    }
}

/// Execute Python code and return the resulting object
/// The code should return an object with init/deinit methods
pub fn execute_python_code(code: &str) -> EngineResult<pyo3::Py<pyo3::PyAny>> {
    let code = code.to_string();
    run_on_native_thread(move || {
        Python::attach(|py| {
            let pkg = py.import("probing").py_context("Python import error")?;

            let result = pkg
                .call_method1("load_extension", (code.as_str(),))
                .py_context("Error loading Python plugin")?;

            if !result
                .hasattr("init")
                .py_context("Unable to check `init` method")?
            {
                return Err(EngineError::plugin("Plugin must have an `init` method"));
            }

            result
                .call_method0("init")
                .py_context("Error calling `init` method")?;

            log::info!("Python extension loaded successfully: {code}");
            Ok(result.unbind())
        })
    })
}

fn backtrace(tid: Option<i32>) -> anyhow::Result<Vec<CallFrame>> {
    SignalTracer.trace(tid)
}

/// Call Python handler through the router system.
///
/// Runs on Tokio's blocking thread pool so the async HTTP/MCP worker is not
/// held while the handler acquires the GIL and executes Python.
async fn call_python_handler(
    path: &str,
    params: &HashMap<String, String>,
    body: &[u8],
) -> EngineResult<Vec<u8>> {
    let path = path.to_string();
    let params = params.clone();
    let body = body.to_vec();
    tokio::task::spawn_blocking(move || call_python_handler_blocking(path, params, body))
        .await
        .map_err(|e| EngineError::plugin(format!("python handler task join failed: {e}")))?
}

fn call_python_handler_blocking(
    path: String,
    params: HashMap<String, String>,
    body: Vec<u8>,
) -> EngineResult<Vec<u8>> {
    run_on_native_thread(move || {
        Python::attach(|py| {
            let router_module = py
                .import("probing.handlers.router")
                .py_context("Failed to import router module")?;

            let handle_func = router_module
                .getattr("handle_request")
                .py_context("Failed to get handle_request function")?;

            let params_dict = pyo3::types::PyDict::new(py);
            for (key, value) in &params {
                params_dict
                    .set_item(key.as_str(), str_to_py(py, value))
                    .py_context_with(|| format!("Failed to set param '{key}'"))?;
            }

            let body_arg = if body.is_empty() {
                py.None()
            } else {
                let body_str = std::str::from_utf8(&body).map_err(|e| {
                    EngineError::plugin(format!("Request body is not valid UTF-8: {e}"))
                })?;
                str_to_py(py, body_str)
            };

            let result = handle_func
                .call1((str_to_py(py, &path), params_dict, body_arg))
                .py_context("Failed to call handle_request")?;

            let result_str: String = match result.extract() {
                Ok(s) => s,
                Err(_) => result
                    .extract::<Vec<u8>>()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .py_context("Failed to extract handler result")?,
            };

            Ok(result_str.into_bytes())
        })
    })
}

fn str_to_py(py: Python, s: &str) -> Py<PyAny> {
    PyString::new(py, s).to_owned().unbind().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_py_ext_list_display() {
        let mut list = PyExtList::default();
        assert_eq!(list.to_string(), "");

        // Add extensions
        Python::attach(|py| {
            let ext1 = py.None();
            let ext2 = py.None();
            list.0.insert("ext1".to_string(), ext1);
            list.0.insert("ext2".to_string(), ext2);
        });

        let display = list.to_string();
        assert!(display.contains("ext1") || display.contains("ext2"));
    }

    #[test]
    fn test_str_to_py() {
        Python::attach(|py| {
            let py_obj = str_to_py(py, "test_string");
            let extracted: String = py_obj.extract(py).unwrap();
            assert_eq!(extracted, "test_string");
        });
    }
}
