//! Unified Python / native (C++ / Rust) stack merge for all tracers.
//!
//! Python frames always come from the eval-frame hook
//! ([`crate::features::stacktrace::tracers::vm`] / `PYSTACKS`).
//! Native frames are spliced at CPython eval-frame boundaries
//! (`_PyEval_EvalFrameDefault` / `EvalFrameEx`).

use std::collections::HashSet;

use lazy_static::lazy_static;
use probing_proto::prelude::CallFrame;

lazy_static! {
    static ref WHITELISTED_PREFIXES: HashSet<&'static str> = {
        const PREFIXES: &[&str] = &[
            "time",
            "sys",
            "gc",
            "os",
            "unicode",
            "thread",
            "stringio",
            "sre",
            "PyGilState",
            "PyThread",
            "lock",
        ];
        PREFIXES.iter().copied().collect()
    };
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum MergeAction {
    /// Drop interpreter trampoline noise.
    Drop,
    /// Keep the native frame as-is.
    KeepNative,
    /// Replace with the next Python frame from the eval hook.
    SplicePython,
}

fn frame_symbol(frame: &CallFrame) -> &str {
    match frame {
        CallFrame::CFrame { func, .. } | CallFrame::PyFrame { func, .. } => func,
    }
}

/// CPython eval splice point in a native stack tower.
pub fn is_eval_frame_name(name: &str) -> bool {
    let mut tokens = name.split(['_', '.']).filter(|s| !s.is_empty());
    matches!(tokens.next(), Some("PyEval"))
        && matches!(
            tokens.next(),
            Some("EvalFrameDefault") | Some("EvalFrameEx")
        )
}

pub fn is_eval_frame(frame: &CallFrame) -> bool {
    is_eval_frame_name(frame_symbol(frame))
}

/// Our eval-frame hook trampoline; dropped from mixed stacks as noise.
pub fn is_interp_shim(frame: &CallFrame) -> bool {
    frame_symbol(frame).contains("rust_eval_frame")
}

/// CPython call-protocol / extension-init noise between user `[py]` frames.
///
/// Kept out of flamegraphs so Distributed stacks do not fan out under
/// `_PyObject_Vectorcall` / probing's `_PyInit__core` after enabling pprof.
fn is_cpython_interpreter_noise(name: &str) -> bool {
    if is_eval_frame_name(name) {
        return false;
    }
    // CPython extension module init (Darwin often shows a leading `_`).
    if name.contains("PyInit_") || name.contains("PyInit__") {
        return true;
    }
    name.contains("vectorcall")
        || name.contains("Vectorcall")
        || name.starts_with("_PyObject_")
        || name.starts_with("PyObject_")
        || name.starts_with("slot_tp_")
        || name.starts_with("_PyFunction_")
        || name.starts_with("cfunction_")
        || name.starts_with("method_vectorcall")
        || name.starts_with("_method_vectorcall")
        || name.starts_with("_PyEval_")
        || name.starts_with("PyEval_")
}

fn merge_action(frame: &CallFrame) -> MergeAction {
    if is_interp_shim(frame) {
        return MergeAction::Drop;
    }
    if is_interpreter_startup_native(frame_symbol(frame)) {
        return MergeAction::Drop;
    }
    if is_eval_frame(frame) {
        return MergeAction::SplicePython;
    }
    if is_cpython_interpreter_noise(frame_symbol(frame)) {
        return MergeAction::Drop;
    }
    let symbol = frame_symbol(frame);
    let mut tokens = symbol.split(['_', '.']).filter(|s| !s.is_empty());
    match tokens.next() {
        Some("PyEval") => MergeAction::Drop,
        Some(prefix) if WHITELISTED_PREFIXES.contains(prefix) => MergeAction::KeepNative,
        _ => MergeAction::KeepNative,
    }
}

/// Demangle a native symbol name; returns `(display_name, lang_tag)`.
pub fn demangle_native_symbol(raw_name: &str) -> (String, Option<&'static str>) {
    if let Ok(d) = rustc_demangle::try_demangle(raw_name) {
        return (d.to_string(), Some("rust"));
    }
    // macOS C ABI adds a leading `_` to Rust v0 mangling (`_R...` -> `__R...`).
    if raw_name.starts_with("__R") {
        if let Ok(d) = rustc_demangle::try_demangle(&raw_name[1..]) {
            return (d.to_string(), Some("rust"));
        }
    }
    if let Some(demangled) = cpp_demangle::Symbol::new(raw_name)
        .ok()
        .and_then(|sym| sym.demangle().ok())
    {
        return (demangled, Some("cpp"));
    }
    (raw_name.to_string(), None)
}

/// Merge Python eval-hook frames with a native stack captured leaf -> root.
///
/// `python_outer_to_inner` follows `PYSTACKS` / `get_python_stacks_raw` order
/// (outermost Python frame first). `native_leaf_to_root` starts at the
/// interrupted PC and walks toward the root.
///
/// Returns root -> leaf (caller -> callee), best-effort when walks truncate.
pub fn merge_python_native_stacks(
    python_outer_to_inner: &[CallFrame],
    native_leaf_to_root: &[CallFrame],
) -> Vec<CallFrame> {
    if native_leaf_to_root.is_empty() {
        return python_outer_to_inner.to_vec();
    }
    if python_outer_to_inner.is_empty() {
        let mut out: Vec<CallFrame> = native_leaf_to_root
            .iter()
            .filter(|f| !is_interp_shim(f))
            .filter(|f| !is_interpreter_startup_native(frame_symbol(f)))
            .cloned()
            .collect();
        // Match the root -> leaf order returned by the merge paths below.
        out.reverse();
        return out;
    }

    // Innermost Python aligns with the deepest eval frame in the native walk.
    let py_inner_to_outer: Vec<CallFrame> = python_outer_to_inner.iter().rev().cloned().collect();
    let eval_count = native_leaf_to_root
        .iter()
        .filter(|f| is_eval_frame(f))
        .count();

    let mut merged_leaf_to_root: Vec<CallFrame> = Vec::new();

    if eval_count > 0 {
        let mut pi = 0usize;
        for frame in native_leaf_to_root {
            match merge_action(frame) {
                MergeAction::Drop => {}
                MergeAction::SplicePython => {
                    if let Some(py) = py_inner_to_outer.get(pi) {
                        merged_leaf_to_root.push(py.clone());
                    } else {
                        merged_leaf_to_root.push(frame.clone());
                    }
                    pi += 1;
                }
                MergeAction::KeepNative => merged_leaf_to_root.push(frame.clone()),
            }
        }
        if pi < py_inner_to_outer.len() {
            merged_leaf_to_root.extend_from_slice(&py_inner_to_outer[pi..]);
        }
    } else {
        // No eval-frame splice anchors (typical for SyncWalk mid-hook): do not
        // concatenate the entire native tower under Python — that was the
        // `_PyInit__core` / vectorcall soup on Distributed after enabling pprof.
        return python_outer_to_inner.to_vec();
    }

    merged_leaf_to_root.reverse();
    merged_leaf_to_root
}

/// Stdlib bootstrap frames that should not anchor a flamegraph root.
fn is_stdlib_bootstrap_py_segment(seg: &str) -> bool {
    if !seg.starts_with("[py]") {
        return false;
    }
    seg.contains("builtins.py")
        || seg.contains("<frozen")
        || seg.contains("importlib/")
        || seg.contains("runpy.py")
        || seg.contains("zipimport.py")
        // platform.<module> often appears above user main_worker via import side-effects.
        || seg.contains("(platform.py:")
        || seg.ends_with("(platform.py)")
}

/// CPython main / import bootstrap that sometimes appears above user `[py]` frames.
fn is_interpreter_startup_native(name: &str) -> bool {
    name.starts_with("_Py_RunMain")
        || name.starts_with("pymain_")
        || name.starts_with("_pymain_")
        || name.starts_with("_PyRun_")
        || name.starts_with("PyRun_")
        || name.starts_with("_run_mod")
        || name.starts_with("run_mod")
        || name == "Py_BytesMain"
        || name == "_Py_BytesMain"
}

/// Align folded paths for merge: drop leading native bootstrap and stdlib `[py]` noise.
///
/// SIGPROF samples occasionally include `_Py_RunMain` → … → user code while others
/// start directly at `[py] <module> (script.py)`. Without this, identical stacks fork.
pub fn canonicalize_folded_segments(segments: &[String]) -> Vec<String> {
    if segments.is_empty() {
        return Vec::new();
    }
    let py_start = segments
        .iter()
        .position(|s| s.starts_with("[py]"))
        .unwrap_or(segments.len());
    let mut out: Vec<String> = segments[py_start..].to_vec();
    while out.len() > 1 && is_stdlib_bootstrap_py_segment(&out[0]) {
        out.remove(0);
    }
    if out.is_empty() {
        segments.to_vec()
    } else {
        out
    }
}

/// Format merged frames as folded flamegraph segments (root -> leaf), canonicalized.
pub fn merged_frames_to_folded_segments(frames: &[CallFrame]) -> Vec<String> {
    let raw: Vec<String> = frames
        .iter()
        .map(|frame| match frame {
            CallFrame::PyFrame {
                file, func, lineno, ..
            } => {
                let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
                format!("[py] {func} ({base}:{lineno})")
            }
            CallFrame::CFrame { func, .. } => func.clone(),
        })
        .collect();
    canonicalize_folded_segments(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py(file: &str, func: &str, line: i64) -> CallFrame {
        CallFrame::PyFrame {
            file: file.into(),
            func: func.into(),
            lineno: line,
            locals: Default::default(),
        }
    }

    fn c(func: &str) -> CallFrame {
        CallFrame::CFrame {
            ip: "0x1".into(),
            file: String::new(),
            func: func.into(),
            lineno: 0,
            lang: Some("cpp".into()),
        }
    }

    #[test]
    fn canonicalize_drops_interpreter_bootstrap_before_user_py() {
        let raw = vec![
            "_Py_RunMain".to_string(),
            "_pymain_run_file".to_string(),
            "[py] <module> (imagenet_with_span.py:1)".to_string(),
            "[py] main (imagenet_with_span.py:316)".to_string(),
        ];
        let out = canonicalize_folded_segments(&raw);
        assert_eq!(
            out,
            vec![
                "[py] <module> (imagenet_with_span.py:1)".to_string(),
                "[py] main (imagenet_with_span.py:316)".to_string(),
            ]
        );
    }

    #[test]
    fn canonicalize_is_idempotent_for_user_rooted_paths() {
        let user = vec![
            "[py] <module> (train.py:1)".to_string(),
            "[py] train (train.py:10)".to_string(),
        ];
        assert_eq!(canonicalize_folded_segments(&user), user);
    }

    #[test]
    fn canonicalize_drops_platform_import_root_before_user() {
        // Observed distributed fork: platform.py:<module> vs imagenet script root.
        let raw = vec![
            "[py] <module> (platform.py:1)".to_string(),
            "[py] main_worker (imagenet_with_span.py:361)".to_string(),
            "[py] train (imagenet_with_span.py:659)".to_string(),
        ];
        assert_eq!(
            canonicalize_folded_segments(&raw),
            vec![
                "[py] main_worker (imagenet_with_span.py:361)".to_string(),
                "[py] train (imagenet_with_span.py:659)".to_string(),
            ]
        );
    }

    #[test]
    fn eval_frames_splice_python_innermost_first() {
        let native = vec![
            c("leaf_native"),
            c("_PyEval_EvalFrameDefault"),
            c("caller_native"),
        ];
        // Outer -> inner; only one eval splice point, so leftover outer sits at root.
        let python = vec![py("outer.py", "outer", 1), py("inner.py", "inner", 2)];
        let merged = merge_python_native_stacks(&python, &native);
        assert_eq!(
            merged,
            vec![
                py("outer.py", "outer", 1),
                c("caller_native"),
                py("inner.py", "inner", 2),
                c("leaf_native"),
            ]
        );
    }

    #[test]
    fn no_eval_anchor_prefers_python_over_native_soup() {
        let native = vec![
            c("_PyObject_Vectorcall"),
            c("_PyInit__core"),
            c("slot_tp_call"),
        ];
        let python = vec![
            py("imagenet_with_span.py", "train", 659),
            py("module.py", "_call_impl", 10),
        ];
        let merged = merge_python_native_stacks(&python, &native);
        assert_eq!(merged, python);
    }

    #[test]
    fn drops_vectorcall_and_extension_init_between_eval_splices() {
        let native = vec![
            c("torch_kernel"),
            c("_PyObject_Vectorcall"),
            c("_PyEval_EvalFrameDefault"),
            c("_PyInit__core"),
            c("caller_native"),
        ];
        let python = vec![py("a.py", "a", 1)];
        let merged = merge_python_native_stacks(&python, &native);
        assert_eq!(
            merged,
            vec![c("caller_native"), py("a.py", "a", 1), c("torch_kernel")]
        );
    }

    #[test]
    fn drops_interp_shim() {
        // Native input is leaf -> root; output must be root -> leaf.
        let native = vec![c("leaf"), c("rust_eval_frame"), c("root")];
        let merged = merge_python_native_stacks(&[], &native);
        assert_eq!(merged, vec![c("root"), c("leaf")]);
    }

    #[test]
    fn pure_python_passthrough() {
        let python = vec![py("a.py", "a", 1), py("b.py", "b", 2)];
        let merged = merge_python_native_stacks(&python, &[]);
        assert_eq!(merged, python);
    }
}
