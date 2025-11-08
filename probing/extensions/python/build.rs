fn main() {
    // Try to find pyenv's Python first, before pyo3_build_config reads the environment
    if std::env::var("PYTHON_SYS_EXECUTABLE").is_err() && std::env::var("PYO3_PYTHON").is_err() {
        if let Some(python_path) = find_pyenv_python() {
            // Set both environment variables for PyO3
            std::env::set_var("PYTHON_SYS_EXECUTABLE", &python_path);
            std::env::set_var("PYO3_PYTHON", &python_path);
            println!("cargo:warning=Using pyenv Python: {}", python_path);
        }
    }

    pyo3_build_config::use_pyo3_cfgs();
    pyo3_build_config::add_extension_module_link_args();
}

fn find_pyenv_python() -> Option<String> {
    // First, try to use pyenv which command
    if let Ok(output) = std::process::Command::new("pyenv")
        .args(&["which", "python3"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if std::path::Path::new(&path).exists() {
                return Some(path);
            }
        }
    }

    // Try to read .python-version file in current directory or parent directories
    let mut current_dir = std::env::current_dir().ok()?;
    loop {
        let python_version_file = current_dir.join(".python-version");
        if python_version_file.exists() {
            if let Ok(version) = std::fs::read_to_string(&python_version_file) {
                let version = version.trim();
                // Try PYENV_ROOT
                if let Ok(pyenv_root) = std::env::var("PYENV_ROOT") {
                    let python_path = format!("{}/versions/{}/bin/python3", pyenv_root, version);
                    if std::path::Path::new(&python_path).exists() {
                        return Some(python_path);
                    }
                }
                // Try ~/.pyenv
                if let Ok(home) = std::env::var("HOME") {
                    let python_path = format!("{}/.pyenv/versions/{}/bin/python3", home, version);
                    if std::path::Path::new(&python_path).exists() {
                        return Some(python_path);
                    }
                }
            }
        }

        // Move to parent directory
        match current_dir.parent() {
            Some(parent) => current_dir = parent.to_path_buf(),
            None => break,
        }
    }

    None
}
