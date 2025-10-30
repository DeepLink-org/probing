use dioxus::prelude::*;
use probing_proto::prelude::Value;
use std::collections::HashMap;

#[component]
pub fn ValueList(variables: HashMap<String, Value>) -> Element {
    rsx! {
        div {
            class: "overflow-x-auto",
            table {
                class: "min-w-full divide-y divide-gray-200 dark:divide-gray-700",
                thead {
                    class: "bg-gray-50 dark:bg-gray-800",
                    tr {
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider",
                            "#"
                        }
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider",
                            "Name"
                        }
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider",
                            "Value"
                        }
                    }
                }
                tbody {
                    class: "bg-white dark:bg-gray-900 divide-y divide-gray-200 dark:divide-gray-700",
                    for (name, value) in variables {
                        tr {
                            td {
                                class: "px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-900 dark:text-white",
                                "{value.id}"
                            }
                            td {
                                class: "px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-900 dark:text-white",
                                "{name}"
                            }
                            td {
                                class: "px-6 py-4 text-sm text-gray-900 dark:text-white break-all",
                                if let Some(val) = &value.value {
                                    "{val}"
                                } else {
                                    "None"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
