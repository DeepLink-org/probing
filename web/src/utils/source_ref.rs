//! Parse `path:line` references and slice remote source for display.

use std::collections::HashSet;
use std::path::Path;

/// A file path optionally pinned to a line number (1-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRef {
    pub path: String,
    pub line: Option<u32>,
}

/// Window of source lines around a highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSlice {
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub highlight_line: Option<u32>,
}

const CONTEXT_LINES: usize = 30;

pub const DEFAULT_SOURCE_CONTEXT: usize = CONTEXT_LINES;

/// Extract unique `file:line` references from free-form text (assistant replies, logs).
pub fn extract_source_refs(text: &str) -> Vec<SourceRef> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for token in text.split_whitespace() {
        let token = token.trim_matches(|c: char| ".,;:)]}'\"`".contains(c));
        if token.is_empty() {
            continue;
        }
        let Some(colon) = token.rfind(':') else {
            continue;
        };
        if colon == 0 || colon + 1 >= token.len() {
            continue;
        }
        let path = &token[..colon];
        let line_str = &token[colon + 1..];
        if !line_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !looks_like_source_path(path) {
            continue;
        }
        let line = line_str.parse::<u32>().ok();
        let key = format!("{path}:{}", line.unwrap_or(0));
        if seen.insert(key) {
            out.push(SourceRef {
                path: path.to_string(),
                line,
            });
        }
    }

    out
}

fn looks_like_source_path(path: &str) -> bool {
    if path.len() < 3 {
        return false;
    }
    if path.contains('/') || path.starts_with('.') {
        return true;
    }
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "py" | "pyi"
                    | "rs"
                    | "cpp"
                    | "cc"
                    | "cxx"
                    | "c"
                    | "h"
                    | "hpp"
                    | "cu"
                    | "cuh"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "json"
                    | "md"
                    | "sh"
            )
        })
}

/// Parse `GET /apis/files?path=…` into a local path.
pub fn parse_files_api_path(api_path: &str) -> Option<String> {
    if !api_path.contains("/apis/files") {
        return None;
    }
    let query = api_path.split('?').nth(1)?;
    for part in query.split('&') {
        if let Some(value) = part.strip_prefix("path=") {
            return urlencoding::decode(value).ok().map(|s| s.into_owned());
        }
    }
    None
}

pub fn language_class(ext_or_path: &str) -> &'static str {
    let ext = Path::new(ext_or_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(ext_or_path);
    match ext {
        "py" | "pyi" => "python",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "rs" => "rust",
        "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "cu" | "cuh" => "cpp",
        "sh" => "shell",
        "md" => "markdown",
        _ => "text",
    }
}

/// Languages enabled in `web/Cargo.toml` for `dioxus-code` (python, json, toml).
#[cfg(not(target_arch = "wasm32"))]
pub fn highlight_language(path: &str) -> Option<dioxus_code::Language> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "py" | "pyi" => Some(dioxus_code::Language::Python),
        "json" => Some(dioxus_code::Language::Json),
        "toml" => Some(dioxus_code::Language::Toml),
        _ => None,
    }
}

pub fn file_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Slice source around `highlight_line` (1-based). Uses [`CONTEXT_LINES`] when set.
pub fn slice_source(content: &str, highlight_line: Option<u32>, context: usize) -> SourceSlice {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    if total == 0 {
        return SourceSlice {
            text: String::new(),
            start_line: 0,
            end_line: 0,
            total_lines: 0,
            highlight_line,
        };
    }

    let ctx = if context == 0 {
        CONTEXT_LINES
    } else {
        context.max(5)
    };
    let (start, end) = match highlight_line {
        Some(l) if l > 0 => {
            let idx = (l as usize).saturating_sub(1).min(total.saturating_sub(1));
            let s = idx.saturating_sub(ctx);
            let e = (idx + ctx + 1).min(total);
            (s, e)
        }
        _ => (0, total.min(ctx * 2 + 1)),
    };

    SourceSlice {
        text: lines[start..end].join("\n"),
        start_line: start + 1,
        end_line: end,
        total_lines: total,
        highlight_line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_refs_from_text() {
        let refs =
            extract_source_refs("See train.py:142 and /opt/project/model.py:88 for details.");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].path, "train.py");
        assert_eq!(refs[0].line, Some(142));
    }

    #[test]
    fn slice_around_line() {
        let src = (1..=100)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let slice = slice_source(&src, Some(50), 10);
        assert_eq!(slice.start_line, 40);
        assert_eq!(slice.end_line, 61);
        assert!(slice.text.contains("line 50"));
    }

    #[test]
    fn parse_files_api() {
        assert_eq!(
            parse_files_api_path("/apis/files?path=%2Ftmp%2Fa.py"),
            Some("/tmp/a.py".to_string())
        );
    }
}
