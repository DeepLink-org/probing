//! Distributed / multi-rank folded-stack merge helpers.
//!
//! Stack merge is implemented on [`crate::features::stacktrace::FoldedStacks`];
//! this module adapts classic `"path count"` lines and [`AttributedFoldedLine`].

use super::{parse_folded_line, AttributedFoldedLine};
use crate::features::stacktrace::fold::{folded_from_line, merge_folded_attributed, FoldedStacks};
use crate::features::stacktrace::snapshot::StackSource;

/// Drop root `all` and per-process `thread-*` prefixes so identical stacks merge across ranks.
pub fn normalize_distributed_stack_folded_line(line: &str) -> Option<String> {
    let stack = folded_from_line(line, StackSource::Unknown)?;
    let merged = merge_folded_attributed(&[(None, vec![stack])]);
    let f = merged.into_iter().next()?;
    Some(f.to_folded_line())
}

/// Keep only `[py] …` segments so native/C frames do not fork the flamegraph.
pub fn filter_folded_line_python_only(line: &str) -> Option<String> {
    let stack = folded_from_line(line, StackSource::Unknown)?;
    let py = stack.python_only()?;
    Some(py.to_folded_line())
}

pub fn filter_folded_lines_python_only(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| filter_folded_line_python_only(line))
        .collect()
}

/// Keep only `[py]` segments while preserving rank attribution.
pub fn filter_attributed_folded_lines_python_only(
    lines: &[AttributedFoldedLine],
) -> Vec<AttributedFoldedLine> {
    lines
        .iter()
        .filter_map(|line| {
            let stack = FoldedStacks {
                tid: 0,
                source: StackSource::Unknown,
                segments: line.path.clone(),
                count: line.count,
                ranks: line.ranks.clone(),
            };
            let py = stack.python_only()?;
            Some(AttributedFoldedLine {
                path: py.segments,
                count: py.count,
                ranks: py.ranks,
            })
        })
        .collect()
}

/// Merge folded stack lines across ranks after normalization / canonicalize.
pub fn merge_distributed_stack_folded_line_sets(sets: &[Vec<String>]) -> Vec<String> {
    let attributed: Vec<(Option<i32>, Vec<String>)> =
        sets.iter().map(|lines| (None, lines.clone())).collect();
    merge_distributed_stack_attributed(&attributed)
        .into_iter()
        .map(|line| format!("{} {}", line.path.join(";"), line.count))
        .collect()
}

/// Merge per-rank folded stacks, summing counts and collecting contributing ranks.
pub fn merge_distributed_stack_attributed(
    sets: &[(Option<i32>, Vec<String>)],
) -> Vec<AttributedFoldedLine> {
    let typed: Vec<(Option<i32>, Vec<FoldedStacks>)> = sets
        .iter()
        .map(|(rank, lines)| {
            let stacks = lines
                .iter()
                .filter_map(|line| folded_from_line(line, StackSource::Unknown))
                .collect();
            (*rank, stacks)
        })
        .collect();
    merge_folded_attributed(&typed)
        .into_iter()
        .map(|f| AttributedFoldedLine {
            path: f.segments,
            count: f.count,
            ranks: f.ranks,
        })
        .collect()
}

/// Merge folded `"stack;path count"` lines from multiple ranks; identical paths sum counts.
pub fn merge_folded_line_sets(sets: &[Vec<String>]) -> Vec<String> {
    use std::collections::HashMap;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for lines in sets {
        for line in lines {
            if let Some((path, count)) = parse_folded_line(line) {
                let key = path.join(";");
                *counts.entry(key).or_insert(0) += count;
            }
        }
    }
    let mut merged: Vec<String> = counts
        .into_iter()
        .map(|(path, count)| format!("{path} {count}"))
        .collect();
    merged.sort();
    merged
}
