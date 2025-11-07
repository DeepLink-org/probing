use dioxus::prelude::*;
use dioxus_router::Link;
use probing_proto::prelude::Process;
use crate::components::table_view::TableView; // using inline Tailwind inside TableView
use crate::app::Route;

#[component]
pub fn ProcessCard(process: Process) -> Element {
    let data = vec![
        vec!["Process ID(pid)".to_string(), process.pid.to_string()],
        vec!["Executable Path(exe)".to_string(), process.exe.to_string()],
        vec!["Command Line(cmd)".to_string(), process.cmd.to_string()],
        vec!["Current Working Directory(cwd)".to_string(), process.cwd.to_string()],
    ];

    rsx! {
        TableView {
            headers: vec!["name".to_string(), "value".to_string()],
            data: data
        }
    }
}

#[component]
pub fn ThreadsCard(threads: Vec<u64>) -> Element {
    rsx! {
        div {
            class: "flex flex-wrap gap-2",
            div {
                class: "text-xs text-gray-500 mb-2",
                "Debug: ThreadsCard received {threads.len()} threads"
            }
            if threads.is_empty() {
                span {
                    class: "text-gray-500 italic",
                    "No threads found"
                }
            } else {
                for tid in threads {
                    Link {
                        to: Route::StackPage {},
                        button {
                            class: "px-3 py-1 text-sm bg-blue-100 text-blue-800 hover:bg-blue-200 rounded-md transition-colors",
                            "{tid}"
                        }
                    }
                }
            }
        }
    }
}
