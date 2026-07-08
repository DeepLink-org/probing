use pyo3::ffi::c_str;
use pyo3::{
    types::{PyDict, PyDictMethods},
    Py, PyAny, Python,
};

use crate::repl::python_repl::PythonConsole;

pub struct NativePythonConsole {
    /// None if import or debug_console lookup failed (avoids panic in Default).
    console: Option<Py<PyAny>>,
}

impl Default for NativePythonConsole {
    #[inline(never)]
    fn default() -> Self {
        let console = Python::attach(|py| {
            let global = PyDict::new(py);
            let code = c_str!(
                "from probing.repl import get_debug_console; debug_console = get_debug_console()"
            );
            if py.run(code, Some(&global), Some(&global)).is_err() {
                log::warn!("probing.repl import failed; REPL will be unavailable");
                return None;
            }
            match global.get_item("debug_console") {
                Ok(Some(console)) => Some(console.unbind()),
                Ok(None) => {
                    log::warn!("debug_console not found after import; REPL will be unavailable");
                    None
                }
                Err(e) => {
                    log::warn!(
                        "error initializing console (debug_console lookup failed): {e}; REPL will be unavailable"
                    );
                    None
                }
            }
        });
        Self { console }
    }
}

impl PythonConsole for NativePythonConsole {
    fn try_execute(&mut self, cmd: String) -> Option<String> {
        let console = self.console.as_ref()?;
        Python::attach(|py| match console.call_method1(py, "push", (cmd,)) {
            Ok(obj) => {
                if obj.is_none(py) {
                    None
                } else {
                    Some(obj.to_string())
                }
            }
            Err(err) => Some(err.to_string()),
        })
    }
}
