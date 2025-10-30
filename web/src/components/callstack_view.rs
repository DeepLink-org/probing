use dioxus::prelude::*;
use probing_proto::prelude::CallFrame;
use crate::components::value_list::ValueList;
use crate::components::collapsible_card::CollapsibleCardWithIcon;

#[component]
pub fn CallStackView(callstack: CallFrame) -> Element {
    match callstack {
        CallFrame::CFrame { ip, file, func, lineno } => {
            let key = format!("{ip}: {func} @ {file}: {lineno}");
            rsx! {
                CollapsibleCardWithIcon {
                    title: key,
                    icon: rsx! {
                        svg {
                            class: "w-4 h-4 text-blue-600 dark:text-blue-400",
                            view_box: "0 0 24 24",
                            fill: "currentColor",
                            path {
                                d: "M22.39 12c0-1.57-.3-3.07-.84-4.44-.55-1.37-1.33-2.58-2.33-3.59s-2.22-1.78-3.59-2.33C13.07 1.3 11.57 1 10 1S6.93 1.3 5.56 1.84C4.19 2.39 2.98 3.17 1.97 4.17s-1.78 2.22-2.33 3.59C1.3 8.93 1 10.43 1 12s.3 3.07.84 4.44c.55 1.37 1.33 2.58 2.33 3.59s2.22 1.78 3.59 2.33C6.93 22.7 8.43 23 10 23s3.07-.3 4.44-.84c1.37-.55 2.58-1.33 3.59-2.33s1.78-2.22 2.33-3.59c.54-1.37.84-2.87.84-4.44zM10 21c-2.76 0-5.1-.96-7.04-2.56C.96 16.5 0 14.16 0 11.4s.96-5.1 2.56-7.04C4.5 2.96 6.84 2 9.6 2s5.1.96 7.04 2.56C18.04 6.5 19 8.84 19 11.6s-.96 5.1-2.56 7.04C14.5 20.04 12.16 21 9.4 21H10z"
                            }
                            path {
                                d: "M15.5 8.5c-.28 0-.5-.22-.5-.5s.22-.5.5-.5.5.22.5.5-.22.5-.5.5zM8.5 8.5c-.28 0-.5-.22-.5-.5s.22-.5.5-.5.5.22.5.5-.22.5-.5.5zM12 17c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z"
                            }
                        }
                    },
                    pre {
                        class: "text-sm text-gray-600 dark:text-gray-400 font-mono bg-gray-50 dark:bg-gray-800 p-3 rounded",
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
                        svg {
                            class: "w-4 h-4 text-green-600 dark:text-green-400",
                            view_box: "0 0 24 24",
                            fill: "currentColor",
                            path {
                                d: "M14.25 0h-8.5A5.75 5.75 0 0 0 0 5.75v12.5A5.75 5.75 0 0 0 5.75 24h8.5A5.75 5.75 0 0 0 20 18.25V5.75A5.75 5.75 0 0 0 14.25 0zm-8.5 2h8.5A3.75 3.75 0 0 1 18 5.75v12.5A3.75 3.75 0 0 1 14.25 22h-8.5A3.75 3.75 0 0 1 2 18.25V5.75A3.75 3.75 0 0 1 5.75 2z"
                            }
                            path {
                                d: "M7.5 6.5a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm9 0a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm-9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm9 0a1 1 0 1 1-2 0 1 1 0 0 1 2 0z"
                            }
                            path {
                                d: "M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7zm0 5a1.5 1.5 0 1 1 0-3 1.5 1.5 0 0 1 0 3z"
                            }
                        }
                    },
                    div {
                        class: "space-y-3",
                        div {
                            class: "text-sm",
                            span {
                                class: "font-medium text-gray-700 dark:text-gray-300",
                                "local: "
                            }
                            span {
                                class: "font-mono text-sm text-gray-600 dark:text-gray-400",
                                "{func} @ "
                                a {
                                    href: "{url}",
                                    target: "_blank",
                                    class: "text-blue-600 dark:text-blue-400 hover:underline",
                                    "{file}"
                                }
                                ": {lineno}"
                            }
                        }
                        if !locals.is_empty() {
                            div {
                                class: "mt-3",
                                h4 {
                                    class: "text-sm font-medium text-gray-700 dark:text-gray-300 mb-2",
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