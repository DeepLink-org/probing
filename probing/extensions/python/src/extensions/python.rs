use std::collections::HashMap;
use std::fmt::Display;

use anyhow::Result;
use async_trait::async_trait;

use probing_core::core::EngineCall;
use probing_core::core::EngineDatasource;
use probing_core::core::EngineError;
use probing_core::core::EngineExtension;
use probing_core::core::EngineExtensionOption;
use probing_core::core::Maybe;
use probing_proto::prelude::CallFrame;
use pyo3::types::PyAnyMethods;
use pyo3::Python;

pub use exttbls::ExternalTable;
pub use exttbls::PyExternalTableConfig;
pub use tbls::PythonPlugin;

use crate::features::stack_tracer::{SignalTracer, StackTracer};
use crate::python::enable_crash_handler;
use crate::python::enable_monitoring;
use crate::python::CRASH_HANDLER;
use crate::repl::PythonRepl;

/// Define a static Mutex for the backtrace function
mod exttbls;
mod stack;
mod tbls;

pub use stack::get_python_stacks;
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
#[derive(Debug, EngineExtension)]
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

    tracer: Box<dyn StackTracer>,
}

impl Default for PythonExt {
    fn default() -> Self {
        Self {
            crash_handler: Default::default(),
            monitoring: Default::default(),
            enabled: Default::default(),
            disabled: Default::default(),
            tracer: Box::new(SignalTracer),
        }
    }
}

#[async_trait]
impl EngineCall for PythonExt {
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
        if path == "callstack" {
            let frames = if params.contains_key("tid") {
                let tid = params
                    .get("tid")
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or_else(|| {
                        log::warn!("Invalid tid parameter, using None");
                        0
                    });
                self.tracer.trace(Some(tid))
            } else {
                self.tracer.trace(None)
            }
            .map_err(|e| {
                log::error!("Failed to get call stack: {e}");
                EngineError::PluginError(format!("Failed to get call stack: {e}"))
            })?;
            return serde_json::to_vec(&frames).map_err(|e| {
                log::error!("Failed to serialize call stack: {e}");
                EngineError::PluginError(format!("Failed to serialize call stack: {e}"))
            });
        }
        if path == "eval" {
            let code = String::from_utf8(body.to_vec()).map_err(|e| {
                log::error!("Failed to convert body to UTF-8 string: {e}");
                EngineError::PluginError(format!("Failed to convert body to UTF-8 string: {e}"))
            })?;

            log::debug!("Python eval code: {code}");

            let mut repl = PythonRepl::default();
            return Ok(repl.process(code.as_str()).unwrap_or_default().into_bytes());
        }
        if path == "flamegraph" {
            return Ok(crate::features::torch::flamegraph().into_bytes());
        }
        // Trace API endpoints
        if path == "trace/list" {
            return Python::with_gil(|py| {
                use pyo3::types::PyDict;
                use std::ffi::CString;
                let global = PyDict::new(py);
                let prefix = params.get("prefix").cloned();
                let code = if let Some(prefix) = prefix {
                    format!(
                        r#"
import json
from probing.inspect.trace import list_traceable

prefix = "{}"
result = list_traceable(prefix=prefix)
retval = result if result else "[]"
"#,
                        prefix
                    )
                } else {
                    r#"
import json
from probing.inspect.trace import list_traceable

prefix = None
result = list_traceable(prefix=prefix)
retval = result if result else "[]"
"#
                    .to_string()
                };
                let code_cstr = CString::new(code).map_err(|e| {
                    EngineError::PluginError(format!("Failed to create CString: {e}"))
                })?;
                py.run(code_cstr.as_c_str(), Some(&global), Some(&global))
                    .map_err(|e| {
                        EngineError::PluginError(format!("Failed to list traceable: {e}"))
                    })?;
                match global.get_item("retval") {
                    Ok(result) => {
                        let result_str: String = result.extract().map_err(|e| {
                            EngineError::PluginError(format!("Failed to extract result: {e}"))
                        })?;
                        Ok(result_str.into_bytes())
                    }
                    Err(e) => Err(EngineError::PluginError(format!(
                        "Failed to get result: {e}"
                    ))),
                }
            });
        }
        if path == "trace/show" {
            return Python::with_gil(|py| {
                use pyo3::types::PyDict;
                use std::ffi::CString;
                let global = PyDict::new(py);
                let code = r#"
import json
from probing.inspect.trace import show_trace

result = show_trace()
retval = result if result else "[]"
"#;
                let code_cstr = CString::new(code).map_err(|e| {
                    EngineError::PluginError(format!("Failed to create CString: {e}"))
                })?;
                py.run(code_cstr.as_c_str(), Some(&global), Some(&global))
                    .map_err(|e| EngineError::PluginError(format!("Failed to show trace: {e}")))?;
                match global.get_item("retval") {
                    Ok(result) => {
                        let result_str: String = result.extract().map_err(|e| {
                            EngineError::PluginError(format!("Failed to extract result: {e}"))
                        })?;
                        Ok(result_str.into_bytes())
                    }
                    Err(e) => Err(EngineError::PluginError(format!(
                        "Failed to get result: {e}"
                    ))),
                }
            });
        }
        if path == "trace/start" {
            let function = params.get("function").ok_or_else(|| {
                EngineError::PluginError("Missing 'function' parameter".to_string())
            })?;
            let watch = params
                .get("watch")
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let depth = params
                .get("depth")
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(1);

            return Python::with_gil(|py| {
                use pyo3::types::PyDict;
                use std::ffi::CString;
                let global = PyDict::new(py);
                let code = format!(
                    r#"
import json
from probing.inspect.trace import trace

try:
    trace("{}", watch={:?}, depth={})
    result = {{"success": True, "message": "Started tracing {}"}}
except Exception as e:
    result = {{"success": False, "error": str(e)}}
retval = json.dumps(result)
"#,
                    function, watch, depth, function
                );
                let code_cstr = CString::new(code).map_err(|e| {
                    EngineError::PluginError(format!("Failed to create CString: {e}"))
                })?;
                py.run(code_cstr.as_c_str(), Some(&global), Some(&global))
                    .map_err(|e| EngineError::PluginError(format!("Failed to start trace: {e}")))?;
                match global.get_item("retval") {
                    Ok(result) => {
                        let result_str: String = result.extract().map_err(|e| {
                            EngineError::PluginError(format!("Failed to extract result: {e}"))
                        })?;
                        Ok(result_str.into_bytes())
                    }
                    Err(e) => Err(EngineError::PluginError(format!(
                        "Failed to get result: {e}"
                    ))),
                }
            });
        }
        if path == "trace/stop" {
            let function = params.get("function").ok_or_else(|| {
                EngineError::PluginError("Missing 'function' parameter".to_string())
            })?;

            return Python::with_gil(|py| {
                use pyo3::types::PyDict;
                use std::ffi::CString;
                let global = PyDict::new(py);
                let code = format!(
                    r#"
import json
from probing.inspect.trace import untrace

try:
    untrace("{}")
    result = {{"success": True, "message": "Stopped tracing {}"}}
except Exception as e:
    result = {{"success": False, "error": str(e)}}
retval = json.dumps(result)
"#,
                    function, function
                );
                let code_cstr = CString::new(code).map_err(|e| {
                    EngineError::PluginError(format!("Failed to create CString: {e}"))
                })?;
                py.run(code_cstr.as_c_str(), Some(&global), Some(&global))
                    .map_err(|e| EngineError::PluginError(format!("Failed to stop trace: {e}")))?;
                match global.get_item("retval") {
                    Ok(result) => {
                        let result_str: String = result.extract().map_err(|e| {
                            EngineError::PluginError(format!("Failed to extract result: {e}"))
                        })?;
                        Ok(result_str.into_bytes())
                    }
                    Err(e) => Err(EngineError::PluginError(format!(
                        "Failed to get result: {e}"
                    ))),
                }
            });
        }
        if path == "trace/variables" {
            let function = params.get("function");
            let limit = params
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(100);

            return Python::with_gil(|py| {
                use pyo3::types::PyDict;
                use std::ffi::CString;
                let global = PyDict::new(py);
                let code = if let Some(func) = function {
                    format!(
                        r#"
import json
import probing

try:
    # Try with python namespace first, fallback to direct table name
    queries = [
        "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables WHERE function_name = '{}' ORDER BY timestamp DESC LIMIT {}",
        "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM trace_variables WHERE function_name = '{}' ORDER BY timestamp DESC LIMIT {}"
    ]
    df = None
    for query in queries:
        try:
            df = probing.query(query)
            break
        except:
            continue
    if df is None:
        retval = json.dumps({{"error": "Table trace_variables not found"}})
    else:
        result = df.to_dict('records')
        retval = json.dumps(result)
except Exception as e:
    retval = json.dumps({{"error": str(e)}})
"#,
                        func, limit, func, limit
                    )
                } else {
                    format!(
                        r#"
import json
import probing

try:
    # Try with python namespace first, fallback to direct table name
    queries = [
        "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM python.trace_variables ORDER BY timestamp DESC LIMIT {{}}".format({}),
        "SELECT function_name, filename, lineno, variable_name, value, value_type, timestamp FROM trace_variables ORDER BY timestamp DESC LIMIT {{}}".format({})
    ]
    df = None
    for query in queries:
        try:
            df = probing.query(query)
            break
        except:
            continue
    if df is None:
        retval = json.dumps({{"error": "Table trace_variables not found"}})
    else:
        result = df.to_dict('records')
        retval = json.dumps(result)
except Exception as e:
    retval = json.dumps({{"error": str(e)}})
"#,
                        limit, limit
                    )
                };
                let code_cstr = CString::new(code).map_err(|e| {
                    EngineError::PluginError(format!("Failed to create CString: {e}"))
                })?;
                py.run(code_cstr.as_c_str(), Some(&global), Some(&global))
                    .map_err(|e| {
                        EngineError::PluginError(format!("Failed to get variables: {e}"))
                    })?;
                match global.get_item("retval") {
                    Ok(result) => {
                        let result_str: String = result.extract().map_err(|e| {
                            EngineError::PluginError(format!("Failed to extract result: {e}"))
                        })?;
                        Ok(result_str.into_bytes())
                    }
                    Err(e) => Err(EngineError::PluginError(format!(
                        "Failed to get result: {e}"
                    ))),
                }
            });
        }
        Ok("".as_bytes().to_vec())
    }
}

impl EngineDatasource for PythonExt {
    /// Create a plugin instance for the specified namespace
    fn datasrc(
        &self,
        namespace: &str,
        _name: Option<&str>,
    ) -> Option<std::sync::Arc<dyn probing_core::core::Plugin + Sync + Send>> {
        Some(PythonPlugin::create(namespace))
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
        // Extract extension code from Maybe
        let ext = match &enabled {
            Maybe::Nothing => {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_ENABLED.to_string(),
                    enabled.clone().into(),
                ));
            }
            Maybe::Just(e) => e,
        };

        // Check if extension is already loaded
        if self.enabled.0.contains_key(ext) {
            return Err(EngineError::PluginError(format!(
                "Python extension '{ext}' is already enabled"
            )));
        }

        // Execute Python code and get the extension object
        let pyext = execute_python_code(ext)
            .map_err(|e| EngineError::InvalidOptionValue(Self::OPTION_ENABLED.to_string(), e))?;

        // Store the extension
        self.enabled.0.insert(ext.clone(), pyext);
        log::info!("Python extension enabled: {ext}");
        log::debug!("Current enabled extensions: {}", self.enabled);

        Ok(())
    }

    /// Disable a previously enabled Python extension
    fn set_disabled(&mut self, disabled: Maybe<String>) -> Result<(), EngineError> {
        // Extract extension name from Maybe
        let ext = match &disabled {
            Maybe::Nothing => {
                return Err(EngineError::InvalidOptionValue(
                    Self::OPTION_DISABLED.to_string(),
                    disabled.clone().into(),
                ));
            }
            Maybe::Just(e) => e,
        };

        // Remove extension if it exists
        if let Some(pyext) = self.enabled.0.remove(ext) {
            log::info!("Disabling Python extension: {ext}");

            // Call deinit method on extension object
            Python::with_gil(|py| {
                // Call the Python object's deinit method
                match pyext.call_method0(py, "deinit") {
                    Ok(_) => {
                        log::debug!("Extension '{ext}' deinitialized successfully");
                        Ok(())
                    }
                    Err(e) => {
                        let error_msg = format!("Failed to call deinit method on '{ext}': {e}");
                        log::error!("{error_msg}");
                        Err(EngineError::PluginError(error_msg))
                    }
                }
            })
        } else {
            log::debug!("Python extension '{ext}' was not enabled, nothing to disable");
            // Extension wasn't found, not an error
            Ok(())
        }
    }
}

/// Execute Python code and return the resulting object
/// The code should return an object with init/deinit methods
pub fn execute_python_code(code: &str) -> Result<pyo3::Py<pyo3::PyAny>, String> {
    Python::with_gil(|py| {
        let pkg = py.import("probing");

        if pkg.is_err() {
            return Err(format!("Python import error: {}", pkg.err().unwrap()));
        }

        let result = pkg
            .unwrap()
            .call_method1("load_extension", (code,))
            .map_err(|e| format!("Error loading Python plugin: {e}"))?;

        // Verify the object has an init method
        if !result
            .hasattr("init")
            .map_err(|e| format!("Unable to check `init` method: {e}"))?
        {
            return Err("Plugin must have an `init` method".to_string());
        }

        // Initialize the plugin
        result
            .call_method0("init")
            .map_err(|e| format!("Error calling `init` method: {e}"))?;

        log::info!("Python extension loaded successfully: {code}");
        Ok(result.unbind())
    })
}

fn backtrace(tid: Option<i32>) -> Result<Vec<CallFrame>> {
    SignalTracer.trace(tid)
}
