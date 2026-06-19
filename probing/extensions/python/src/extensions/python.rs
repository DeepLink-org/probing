use std::collections::HashMap;
use std::fmt::Display;

use anyhow::Result;
use async_trait::async_trait;

use probing_core::core::EngineError;
use probing_core::core::Maybe;
use probing_core::core::ProbeExtension;
use probing_core::core::ProbeExtensionCall;
use probing_core::core::ProbeExtensionOption;
use probing_core::run_on_native_thread;
use probing_proto::prelude::CallFrame;
use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyString};
use pyo3::Python;

pub use exttbls::ExternalTable;
pub use exttbls::PyExternalTableConfig;
pub use tbls::PythonProbeDataSource;

use crate::features::stack_tracer::{SignalTracer, StackTracer};
use crate::python::enable_crash_handler;
use crate::python::enable_monitoring;
use crate::python::CRASH_HANDLER;

mod exttbls;
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
#[derive(Debug, ProbeExtension)]
pub struct PythonExt {
    /// Path to Python crash handler script (executed when interpreter crashes)
    #[option(aliases = ["crash.handler"])]
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

impl Default for PythonExt {
    fn default() -> Self {
        Self {
            crash_handler: Default::default(),
            monitoring: Default::default(),
            enabled: Default::default(),
            disabled: Default::default(),
        }
    }
}

#[async_trait]
impl ProbeExtensionCall for PythonExt {
    async fn call(
        &self,
        path: &str,
        params: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<Vec<u8>, EngineError> {
        log::debug!(
            "Python extension call - path: {}, params: {:?}, body_size: {}",
            path,
            params,
            body.len()
        );

        let normalized_path = path.trim_start_matches('/');
        call_python_handler(normalized_path, params, body)
    }
}

impl PythonExt {
    /// Set up a Python crash handler
    fn set_crash_handler(&mut self, crash_handler: Maybe<String>) -> Result<(), EngineError> {
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
                    CRASH_HANDLER.lock().unwrap().replace(handler.to_string());
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
    fn set_monitoring(&mut self, monitoring: Maybe<String>) -> Result<(), EngineError> {
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
    fn set_enabled(&mut self, enabled: Maybe<String>) -> Result<(), EngineError> {
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
            return Err(EngineError::PluginError(format!(
                "Python extension '{ext}' is already enabled"
            )));
        }

        let pyext = execute_python_code(ext)
            .map_err(|e| EngineError::InvalidOptionValue(Self::OPTION_ENABLED.to_string(), e))?;

        self.enabled.0.insert(ext.clone(), pyext);
        log::info!("Python extension enabled: {ext}");
        log::debug!("Current enabled extensions: {}", self.enabled);

        Ok(())
    }

    /// Disable a previously enabled Python extension
    fn set_disabled(&mut self, disabled: Maybe<String>) -> Result<(), EngineError> {
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
                        let error_msg =
                            format!("Failed to call deinit method on '{ext_name}': {e}");
                        log::error!("{error_msg}");
                        Err(EngineError::PluginError(error_msg))
                    }
                })
            })
        } else {
            log::debug!("Python extension '{ext}' was not enabled, nothing to disable");
            Ok(())
        }
    }
}

/// Execute Python code and return the resulting object
/// The code should return an object with init/deinit methods
pub fn execute_python_code(code: &str) -> Result<pyo3::Py<pyo3::PyAny>, String> {
    let code = code.to_string();
    run_on_native_thread(move || {
        Python::attach(|py| {
            let pkg = py.import("probing");

            if pkg.is_err() {
                return Err(format!("Python import error: {}", pkg.err().unwrap()));
            }

            let result = pkg
                .unwrap()
                .call_method1("load_extension", (code.as_str(),))
                .map_err(|e| format!("Error loading Python plugin: {e}"))?;

            if !result
                .hasattr("init")
                .map_err(|e| format!("Unable to check `init` method: {e}"))?
            {
                return Err("Plugin must have an `init` method".to_string());
            }

            result
                .call_method0("init")
                .map_err(|e| format!("Error calling `init` method: {e}"))?;

            log::info!("Python extension loaded successfully: {code}");
            Ok(result.unbind())
        })
    })
}

fn backtrace(tid: Option<i32>) -> Result<Vec<CallFrame>> {
    SignalTracer.trace(tid)
}

/// Call Python handler through the router system.
fn call_python_handler(
    path: &str,
    params: &HashMap<String, String>,
    body: &[u8],
) -> Result<Vec<u8>, EngineError> {
    let path = path.to_string();
    let params = params.clone();
    let body = body.to_vec();
    run_on_native_thread(move || {
        Python::attach(|py| {
            let router_module = py.import("probing.handlers.router").map_err(|e| {
                EngineError::PluginError(format!("Failed to import router module: {e}"))
            })?;

            let handle_func = router_module.getattr("handle_request").map_err(|e| {
                EngineError::PluginError(format!("Failed to get handle_request function: {e}"))
            })?;

            let params_dict = pyo3::types::PyDict::new(py);
            for (key, value) in &params {
                params_dict
                    .set_item(key.as_str(), str_to_py(py, value))
                    .map_err(|e| {
                        EngineError::PluginError(format!("Failed to set param '{key}': {e}"))
                    })?;
            }

            let body_arg = if body.is_empty() {
                py.None().into()
            } else {
                let body_str = std::str::from_utf8(&body).map_err(|e| {
                    EngineError::PluginError(format!("Request body is not valid UTF-8: {e}"))
                })?;
                str_to_py(py, body_str)
            };

            let result = handle_func
                .call1((str_to_py(py, &path), params_dict, body_arg))
                .map_err(|e| {
                    EngineError::PluginError(format!("Failed to call handle_request: {e}"))
                })?;

            let result_str: String = result
                .extract()
                .map_err(|e| EngineError::PluginError(format!("Failed to extract result: {e}")))?;

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
