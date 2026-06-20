fn main() {
    pyo3_build_config::use_pyo3_cfgs();
    pyo3_build_config::add_libpython_rpath_link_args();

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        let config = pyo3_build_config::get();
        let lib_name = config.lib_name();
        let lib = lib_name.unwrap_or("python3.12");
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
