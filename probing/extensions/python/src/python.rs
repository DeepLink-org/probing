use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::prelude::*;
use pyo3::{types::PyDict, Bound, Python};

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

/// Install the Python crash handler module (``probing.crash.install``).
pub fn enable_crash_handler() -> anyhow::Result<()> {
    Python::attach(|py| -> anyhow::Result<()> {
        log::debug!("enable crash handler via probing.crash");
        let crash = py.import("probing.crash")?;
        crash.call_method0("install")?;
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

        let code = crate::pycode::get_code(filename).ok_or_else(|| {
            anyhow::anyhow!(
                "monitoring script not found: {filename} (embed under pycode/ or set PROBING_CODE_ROOT)"
            )
        })?;
        run_embedded(py, &code, None, None)
            .map_err(|err| anyhow::anyhow!("error apply monitoring {filename}: {err}"))?;
        Ok(())
    })
}
