//! Call-frame classification and display helpers for the Stacks page.

use probing_proto::prelude::CallFrame;

/// Logical frame kind for UI styling and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Python,
    Rust,
    Cpp,
}

impl FrameKind {
    pub fn label(self) -> &'static str {
        match self {
            FrameKind::Python => "Python",
            FrameKind::Rust => "Rust",
            FrameKind::Cpp => "Native",
        }
    }

    pub fn badge_classes(self) -> &'static str {
        match self {
            FrameKind::Python => "bg-emerald-50 text-emerald-700 border-emerald-200",
            FrameKind::Rust => "bg-orange-50 text-orange-800 border-orange-200",
            FrameKind::Cpp => "bg-blue-50 text-blue-700 border-blue-200",
        }
    }

    pub fn accent_border(self) -> &'static str {
        match self {
            FrameKind::Python => "border-l-emerald-500",
            FrameKind::Rust => "border-l-orange-500",
            FrameKind::Cpp => "border-l-blue-500",
        }
    }

    pub fn icon_classes(self) -> &'static str {
        match self {
            FrameKind::Python => "w-4 h-4 text-emerald-600",
            FrameKind::Rust => "w-4 h-4 text-orange-600",
            FrameKind::Cpp => "w-4 h-4 text-blue-600",
        }
    }
}

pub fn classify_frame(frame: &CallFrame) -> FrameKind {
    match frame {
        CallFrame::PyFrame { .. } => FrameKind::Python,
        CallFrame::CFrame {
            lang: Some(lang), ..
        } if lang == "rust" => FrameKind::Rust,
        CallFrame::CFrame {
            lang: Some(lang), ..
        } if lang == "cpp" => FrameKind::Cpp,
        CallFrame::CFrame { func, file, .. } => {
            if is_rust_native(func, file) {
                FrameKind::Rust
            } else {
                FrameKind::Cpp
            }
        }
    }
}

pub fn mode_for_kind(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::Python => "py",
        FrameKind::Rust => "rust",
        FrameKind::Cpp => "cpp",
    }
}

fn is_rust_native(func: &str, file: &str) -> bool {
    if file.ends_with(".rs") {
        return true;
    }

    if func.contains("{closure") {
        return true;
    }

    if func.contains(" as ") && func.contains(">::") {
        return true;
    }

    if func.starts_with("_R") || func.contains("::_R") {
        return true;
    }

    if func.contains("::") {
        if func.starts_with("std::") {
            return false;
        }
        if file.ends_with(".h") || file.ends_with(".hpp") || file.ends_with(".cc") || file.ends_with(".cpp")
        {
            return false;
        }
        if func.contains("probing_") || func.contains("::_core::") {
            return true;
        }
        if !func.contains('<') && looks_like_rust_path(func) {
            return true;
        }
    }

    false
}

fn looks_like_rust_path(func: &str) -> bool {
    func.split("::").all(|segment| {
        let head = segment.split(['(', '<', '{']).next().unwrap_or(segment);
        !head.is_empty()
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    })
}

pub fn frame_title(frame: &CallFrame) -> String {
    match frame {
        CallFrame::CFrame { func, .. } => func.clone(),
        CallFrame::PyFrame { func, .. } => func.clone(),
    }
}

pub fn frame_location(frame: &CallFrame) -> Option<(String, i64)> {
    match frame {
        CallFrame::CFrame { file, lineno, .. } if !file.is_empty() => {
            Some((file.clone(), *lineno))
        }
        CallFrame::PyFrame { file, lineno, .. } => Some((file.clone(), *lineno)),
        _ => None,
    }
}

pub fn frame_ip(frame: &CallFrame) -> Option<&str> {
    match frame {
        CallFrame::CFrame { ip, .. } => Some(ip.as_str()),
        CallFrame::PyFrame { .. } => None,
    }
}

pub fn count_by_kind(frames: &[CallFrame]) -> (usize, usize, usize) {
    let mut py = 0usize;
    let mut rust = 0usize;
    let mut cpp = 0usize;
    for frame in frames {
        match classify_frame(frame) {
            FrameKind::Python => py += 1,
            FrameKind::Rust => rust += 1,
            FrameKind::Cpp => cpp += 1,
        }
    }
    (py, rust, cpp)
}

pub fn matches_mode(frame: &CallFrame, mode: &str) -> bool {
    match mode {
        "py" => matches!(frame, CallFrame::PyFrame { .. }),
        "rust" => matches!(classify_frame(frame), FrameKind::Rust),
        "cpp" => matches!(classify_frame(frame), FrameKind::Cpp),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_by_rs_file() {
        let frame = CallFrame::CFrame {
            ip: "0x0".into(),
            file: "probing/core/src/runtime.rs".into(),
            func: "probing_core::runtime::block_on".into(),
            lineno: 155,
            lang: Some("rust".into()),
        };
        assert_eq!(classify_frame(&frame), FrameKind::Rust);
    }

    #[test]
    fn detects_cpp_std() {
        let frame = CallFrame::CFrame {
            ip: "0x0".into(),
            file: "vector".into(),
            func: "std::vector<int>::push_back".into(),
            lineno: 0,
            lang: Some("cpp".into()),
        };
        assert_eq!(classify_frame(&frame), FrameKind::Cpp);
    }

    #[test]
    fn detects_python_frame() {
        let frame = CallFrame::PyFrame {
            file: "train.py".into(),
            func: "forward".into(),
            lineno: 12,
            locals: Default::default(),
        };
        assert_eq!(classify_frame(&frame), FrameKind::Python);
    }
}
