use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;
use crate::components::value_list::ValueList;
use crate::components::collapsible_card::CollapsibleCardWithIcon;
use crate::components::icon::Icon;

#[component]
pub fn CallStackView(callstack: CallFrame) -> Element {
    match callstack {
        CallFrame::CFrame { ip, file, func, lineno } => {
            let key = format!("{ip}: {func} @ {file}: {lineno}");
            rsx! {
                CollapsibleCardWithIcon {
                    title: key,
                    icon: rsx! {
                        Icon { icon: &icondata::SiCplusplus, class: "w-4 h-4 text-blue-600" }
                    },
                    pre {
                        class: "text-sm text-gray-600 font-mono bg-gray-50 p-3 rounded",
                        "..."
                    }
                }
            }
        }
        CallFrame::PyFrame { file, func, lineno, locals } => {
            let url = format!("/apis/files?path={}", file);
            let key = format!("{func} @ {file}: {lineno}");
            rsx! {
                CollapsibleCardWithIcon {
                    title: key,
                    icon: rsx! {
                        Icon { icon: &icondata::SiPython, class: "w-4 h-4 text-green-600" }
                    },
                    div {
                        class: "space-y-3",
                        div {
                            class: "text-sm",
                            span {
                                class: "font-medium text-gray-700",
                                "local: "
                            }
                            span {
                                class: "font-mono text-sm text-gray-600",
                                "{func} @ "
                                a {
                                    href: "{url}",
                                    target: "_blank",
                                    class: "text-blue-600 hover:underline",
                                    "{file}"
                                }
                                ": {lineno}"
                            }
                        }
                        if !locals.is_empty() {
                            div {
                                class: "mt-3",
                                h4 {
                                    class: "text-sm font-medium text-gray-700 mb-2",
                                    "Local Variables:"
                                }
                                ValueList {
                                    variables: locals
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
