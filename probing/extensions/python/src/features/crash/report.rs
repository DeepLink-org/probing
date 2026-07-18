//! Human-readable crash report rendering.

use super::handler::CrashEvent;
use super::memory_snapshot;

const WIDTH: usize = 72;

fn rule(title: &str) -> String {
    let head = format!("── {title} ");
    let fill = WIDTH.saturating_sub(head.len());
    format!("{head}{}", "─".repeat(fill))
}

fn rank_label(event: &CrashEvent) -> String {
    if event.world_size > 0 {
        format!("rank {}/{}", event.rank, event.world_size)
    } else if event.rank >= 0 {
        format!("rank {}", event.rank)
    } else {
        "rank ?".into()
    }
}

fn step_line(event: &CrashEvent) -> String {
    if event.global_step > 0 || event.local_step > 0 || event.micro_step > 0 {
        let phase = if event.training_phase.is_empty() {
            String::new()
        } else {
            format!("  phase={}", event.training_phase)
        };
        format!(
            "  step       global={}  local={}  micro={}{phase}",
            event.global_step, event.local_step, event.micro_step
        )
    } else {
        "  step       (not in training loop)".into()
    }
}

fn context_lines(event: &CrashEvent) -> Vec<String> {
    let mut parts = vec![rank_label(event)];
    if event.local_rank >= 0 {
        parts.push(format!("local_rank={}", event.local_rank));
    }
    parts.push(format!("host={}", event.host));
    parts.push(format!("pid={}", event.pid));

    let mut lines = vec![format!("  {}", parts.join("  "))];
    lines.push(step_line(event));
    if !event.active_span.is_empty() {
        lines.push(format!("  span       {}", event.active_span));
    }
    if !event.last_comm_op.is_empty() {
        lines.push(format!("  comm       {}", event.last_comm_op));
    }
    lines.extend(memory_snapshot::report_lines(&event.memory));
    lines
}

fn indent_block(text: &str, max_lines: usize) -> Vec<String> {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.is_empty() {
        return vec![];
    }
    if lines.len() > max_lines {
        let skip = lines.len() - max_lines;
        let mut out = vec![format!("    … {skip} earlier frames omitted …")];
        for line in &lines[skip..] {
            out.push(format!("    {line}"));
        }
        return out;
    }
    lines.iter().map(|line| format!("    {line}")).collect()
}

fn stack_sections(event: &CrashEvent, all_threads_max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let python_hook = matches!(event.kind.as_str(), "python_exception" | "thread_exception");

    if !python_hook && !event.traceback.is_empty() {
        let crash_thread = if event.crash_thread.is_empty() {
            "MainThread"
        } else {
            &event.crash_thread
        };
        lines.push(format!(
            "  crash thread ({crash_thread}) — exception traceback"
        ));
        lines.extend(indent_block(&event.traceback, usize::MAX));
    }

    if !event.thread_stacks.is_empty() {
        lines.push("  all threads — faulthandler snapshot".into());
        lines.extend(indent_block(&event.thread_stacks, all_threads_max));
    } else if !python_hook && !event.native_backtrace.is_empty() {
        lines.push("  native backtrace".into());
        lines.extend(indent_block(&event.native_backtrace, 16));
    }

    lines
}

fn format_event(
    event: &CrashEvent,
    fatal: bool,
    grace_sec: u64,
    hold_active: bool,
    spill_path: Option<&std::path::Path>,
) -> String {
    let title = if fatal {
        format!("PROBING CRASH · {} · FATAL", event.kind)
    } else {
        format!("PROBING CRASH · {} · NON-FATAL", event.kind)
    };
    let stacks_max = if fatal { 64 } else { 48 };
    let short_id = if event.event_id.len() > 16 {
        &event.event_id[..16]
    } else {
        &event.event_id
    };

    let mut lines = vec![
        rule(&title),
        format!("  exception  {}: {}", event.exception_type, event.message),
        format!("  at         {}", event.top_frame),
    ];
    lines.extend(context_lines(event));
    lines.push(format!(
        "  ids        event={short_id}  fingerprint={}",
        event.fingerprint
    ));
    lines.extend(stack_sections(event, stacks_max));

    if fatal {
        if let Some(path) = spill_path {
            lines.push(format!("  spill      {}", path.display()));
        }
        if grace_sec > 0 {
            lines.push(format!(
                "  hold       {grace_sec}s grace — gdb -p {} · touch {} · kill -USR1",
                event.pid,
                super::context::hold_file_path(event.pid)
            ));
        } else if hold_active {
            lines.push("  hold       active — POST /apis/pythonext/crash/release".into());
        }
    } else {
        lines.push(
            "  status     process continues — main thread did NOT exit; training may stall".into(),
        );
        if let Some(path) = spill_path {
            lines.push(format!("  spill      {}", path.display()));
        }
    }

    lines.push("─".repeat(WIDTH));
    lines.join("\n")
}

pub fn print_report(
    event: &CrashEvent,
    grace_sec: u64,
    hold_active: bool,
    spill_path: Option<&std::path::Path>,
) {
    eprintln!(
        "{}",
        format_event(event, true, grace_sec, hold_active, spill_path)
    );
}

pub fn print_summary(event: &CrashEvent, spill_path: Option<&std::path::Path>) {
    eprintln!("{}", format_event(event, false, 0, false, spill_path));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event() -> CrashEvent {
        CrashEvent {
            event_id: "test-event-id".into(),
            timestamp_ns: 1,
            kind: "thread_exception".into(),
            rank: 1,
            local_rank: 1,
            world_size: 4,
            host: "node-a".into(),
            pid: 99,
            exception_type: "RuntimeError".into(),
            message: "demo crash".into(),
            top_frame: "demo.py:19 in boom".into(),
            traceback: String::new(),
            native_backtrace: String::new(),
            crash_thread: "crash-r1".into(),
            thread_stacks: "Thread 0x1 (MainThread)\n".into(),
            fingerprint: "abc123".into(),
            global_step: 42,
            local_step: 42,
            micro_step: 42,
            training_phase: "backward".into(),
            active_span: "train.step".into(),
            last_comm_op: "all_reduce".into(),
            memory: memory_snapshot::MemorySnapshot {
                host_rss_bytes: 1024 * 1024,
                thread_count: 2,
                ..Default::default()
            },
        }
    }

    #[test]
    fn summary_includes_memory_context() {
        let text = format_event(&sample_event(), false, 0, false, None);
        assert!(text.contains("memory"));
        assert!(text.contains("host_rss="));
    }

    #[test]
    fn summary_includes_training_context() {
        let text = format_event(&sample_event(), false, 0, false, None);
        assert!(text.contains("NON-FATAL"));
        assert!(text.contains("global=42"));
        assert!(text.contains("train.step"));
        assert!(!text.contains("memtable"));
    }

    #[test]
    fn hold_active_release_hint_is_http_not_sigusr2() {
        let text = format_event(&sample_event(), true, 0, true, None);
        assert!(text.contains("POST /apis/pythonext/crash/release"));
        assert!(
            !text.contains("USR2") && !text.contains("SIGUSR2"),
            "hold release must not recommend kill -USR2: {text}"
        );
    }
}
