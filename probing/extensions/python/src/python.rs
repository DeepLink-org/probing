use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::Result;
use once_cell::sync::Lazy;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use pyo3::{types::PyDict, Bound, Python};

use crate::pycode::get_code;

pub static CRASH_HANDLER: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
pub static OLD_HANDLER: Lazy<Option<Py<PyAny>>> = Lazy::new(|| None);
pub static PROBING_ENABLED: AtomicBool = AtomicBool::new(false);

fn run_embedded(
    py: Python<'_>,
    source: &str,
    globals: Option<&Bound<'_, PyDict>>,
    locals: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let code = CString::new(source).map_err(|_| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>("embedded Python source contains nul byte")
    })?;
    py.run(&code, globals, locals)
}

fn call_default_handler(typ: Py<PyAny>, value: Py<PyAny>, traceback: Py<PyAny>) -> Result<()> {
    let code = get_code("crash_handler.py").unwrap_or_default();
    Python::attach(|py| -> Result<()> {
        let global = PyDict::new(py);
        run_embedded(py, &code, Some(&global), None)?;
        if let Some(handler) = global.get_item("crash_handler")? {
            let args = PyTuple::new(py, [typ, value, traceback])?;
            handler.call(args, None)?;
        }
        Ok(())
    })
}

fn call_custom_handler(
    handler: &str,
    typ: Py<PyAny>,
    value: Py<PyAny>,
    traceback: Py<PyAny>,
) -> Result<()> {
    Python::attach(|py| -> Result<()> {
        let locals = PyDict::new(py);
        if handler.contains('.') {
            let parts: Vec<&str> = handler.split('.').collect();
            let pkg = py.import(parts[0])?;
            locals.set_item(parts[0], pkg)?;
        }
        locals.set_item("type", typ)?;
        locals.set_item("value", value)?;
        locals.set_item("traceback", traceback)?;
        let ret = (|| {
            let expr = CString::new(handler)?;
            py.eval(&expr, None, Some(&locals))
        })();

        println!("crash handler: {ret:?}");
        Ok(())
    })
}

fn script_basename(py: Python) -> Option<String> {
    let sys = py.import("sys").ok()?;
    let argv = sys.getattr("argv").ok()?;
    let script = argv.get_item(0).ok()?;
    let script_str: String = script.extract().ok()?;
    std::path::Path::new(&script_str)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}

pub fn should_enable_probing() -> bool {
    let probe_value = std::env::var("PROBING_ORIGINAL")
        .or_else(|_| std::env::var("PROBING"))
        .unwrap_or_else(|_| "0".to_string());

    if probe_value == "0" {
        return false;
    }

    let probe_value = if probe_value.starts_with("init:") {
        probe_value
            .split_once('+')
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| "0".to_string())
    } else {
        probe_value
    };

    if probe_value == "0" {
        return false;
    }

    let lower = probe_value.to_lowercase();
    match lower.as_str() {
        "1" | "followed" | "2" | "nested" => true,
        _ if lower.starts_with("regex:") => {
            let Some((_, pattern)) = probe_value.split_once(':') else {
                return false;
            };
            let Ok(regex) = regex::Regex::new(pattern) else {
                return false;
            };
            Python::attach(script_basename)
                .map(|name| regex.is_match(&name))
                .unwrap_or(false)
        }
        _ => Python::attach(script_basename)
            .map(|name| probe_value == name)
            .unwrap_or(false),
    }
}

pub fn is_enabled() -> bool {
    PROBING_ENABLED.load(Ordering::SeqCst)
}

pub fn set_enabled(enabled: bool) {
    PROBING_ENABLED.store(enabled, Ordering::SeqCst);
}

#[pyfunction]
pub fn crash_handler(typ: Py<PyAny>, value: Py<PyAny>, traceback: Py<PyAny>) {
    let handler = CRASH_HANDLER.lock().unwrap().clone();
    log::debug!("call crash handler: {handler:?}");
    if let Some(handler) = handler {
        let ret = match handler.as_str() {
            "default" => call_default_handler(typ, value, traceback),
            handler => call_custom_handler(handler, typ, value, traceback),
        };
        if let Err(err) = ret {
            log::error!("error calling crash handler: {err}");
        }
    }
}

pub fn enable_crash_handler() -> anyhow::Result<()> {
    Python::attach(|py| -> anyhow::Result<()> {
        log::debug!("enable crash handler");
        let sys = py.import("sys")?;
        let func = wrap_pyfunction!(crash_handler, &sys)?;
        sys.setattr("excepthook", func)?;
        Ok(())
    })?;
    Ok(())
}

pub fn enable_monitoring(filename: &str) -> anyhow::Result<()> {
    Python::attach(|py| {
        let ver = py.version_info();
        if ver.major != 3 || ver.minor < 12 {
            return Err(anyhow::anyhow!("Python version must be 3.12+"));
        }

        let filename = if filename == "default" {
            "monitoring.py"
        } else {
            filename
        };

        let code = get_code(filename).ok_or_else(|| {
            anyhow::anyhow!(
                "monitoring script not found: {filename} (embed under pycode/ or set PROBING_CODE_ROOT)"
            )
        })?;
        run_embedded(py, &code, None, None)
            .map_err(|err| anyhow::anyhow!("error apply monitoring {filename}: {err}"))?;
        Ok(())
    })
}
