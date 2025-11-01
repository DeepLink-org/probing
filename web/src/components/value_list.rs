use dioxus::prelude::*;
use probing_proto::prelude::Value;
use std::collections::HashMap;

#[component]
pub fn ValueList(variables: HashMap<String, Value>) -> Element {
    rsx! {
        div {
            class: "overflow-x-auto",
            table {
                class: "min-w-full divide-y divide-gray-200",
                thead {
                    class: "bg-gray-50",
                    tr {
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                            "#"
                        }
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                            "Name"
                        }
                        th {
                            class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider",
                            "Value"
                        }
                    }
                }
                tbody {
                    class: "bg-white divide-y divide-gray-200",
                    for (name, value) in variables {
                        tr {
                            td {
                                class: "px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-900",
                                "{value.id}"
                            }
                            td {
                                class: "px-6 py-4 whitespace-nowrap text-sm font-mono text-gray-900",
                                "{name}"
                            }
                            td {
                                class: "px-6 py-4 text-sm text-gray-900 break-all",
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
