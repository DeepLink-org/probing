//! Python variable tracing page (`api/trace` — live function watch, not distributed spans).

mod active;
mod catalog;
mod dialogs;
mod shared;

use dioxus::prelude::*;

use crate::components::card::Card;
use crate::components::common::AsyncBoundary;
use crate::components::page::{PageContainer, PageTitle};
use crate::components::poll_status::PollStatusBar;
use crate::hooks::{use_page_visible, use_poll_tick_gated};

use active::ActiveTracesPanel;
use catalog::TraceableCatalog;
use dialogs::{RecordsModal, StartTraceDialog};
use shared::{RefreshButton, StartTraceDraft, POLL_MS};

#[component]
pub fn Python() -> Element {
    let visible = use_page_visible();
    let poll = use_poll_tick_gated(POLL_MS, Some(visible));
    let poll_tick = poll();
    let mut refresh_key = use_signal(|| 0u32);
    let mut start_open = use_signal(|| false);
    let mut start_draft = use_signal(|| StartTraceDraft {
        function: String::new(),
        watch: String::new(),
        print_to_terminal: false,
    });
    let mut records_open = use_signal(|| false);
    let mut records_function = use_signal(|| String::new());

    let open_records = EventHandler::new(move |func: String| {
        records_function.set(func);
        records_open.set(true);
    });

    let open_start_dialog = EventHandler::new(move |args: (String, Vec<String>)| {
        let (function, watch_vars) = args;
        start_draft.set(StartTraceDraft {
            function,
            watch: watch_vars.join(", "),
            print_to_terminal: false,
        });
        start_open.set(true);
    });

    rsx! {
        PageContainer {
            PageTitle {
                title: "Python variable tracing".to_string(),
                subtitle: Some(format!(
                    "Watch live function variables — not Distributed Spans or Profiling chrome trace · auto refresh every {}s while tab is visible",
                    POLL_MS / 1000
                )),
                icon: Some(&icondata::SiPython),
                header_right: Some(rsx! {
                    PollStatusBar {
                        interval_secs: POLL_MS / 1000,
                        poll_tick,
                    }
                }),
            }

            div { class: "grid grid-cols-1 lg:grid-cols-12 gap-4 items-start",
                div { class: "lg:col-span-5 min-w-0",
                    Card {
                        title: "Active watches",
                        content_class: Some("p-0"),
                        header_right: Some(rsx! {
                            RefreshButton {
                                onclick: move |_| refresh_key.set(refresh_key() + 1),
                            }
                        }),
                        AsyncBoundary {
                            message: Some("Loading active traces…".to_string()),
                            ActiveTracesPanel {
                                poll,
                                refresh_key,
                                on_view_records: open_records,
                            }
                        }
                    }
                }

                div { class: "lg:col-span-7 min-w-0",
                    Card {
                        title: "Traceable Catalog",
                        content_class: Some("p-0"),
                        AsyncBoundary {
                            message: Some("Loading traceable items…".to_string()),
                            TraceableCatalog {
                                on_start: open_start_dialog,
                            }
                        }
                    }
                }
            }

            if *start_open.read() {
                StartTraceDialog {
                    draft: start_draft,
                    on_close: move |_| start_open.set(false),
                    on_started: move |_| {
                        refresh_key.set(refresh_key() + 1);
                        start_open.set(false);
                    },
                }
            }

            if *records_open.read() {
                RecordsModal {
                    function: records_function(),
                    poll,
                    on_close: move |_| records_open.set(false),
                }
            }
        }
    }
}
