use dioxus::prelude::*;
// Tailwind classes inlined for table view.

#[component]
pub fn TableView(
    headers: Vec<String>,
    data: Vec<Vec<String>>,
    #[props(optional)] on_row_click: Option<EventHandler<usize>>,
) -> Element {
    rsx! {
        div {
            class: "w-full overflow-x-auto border border-gray-200 rounded-lg",

            table {
                class: "w-full border-collapse table-auto",

                thead {
                    tr { class: "bg-gray-50 border-b border-gray-200",
                        for header in headers {
                            th { class: "px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200", {header} }
                        }
                    }
                }

                tbody {
                    for (row_idx, row) in data.iter().enumerate() {
                        tr { 
                            class: if row_idx % 2 == 0 { "bg-white" } else { "bg-gray-50" },
                            onclick: move |_| {
                                if let Some(cb) = on_row_click {
                                    cb.call(row_idx);
                                }
                            },
                            for cell in row {
                                td { class: "px-4 py-2 text-gray-700 border-r border-gray-200", {cell.clone()} }
                            }
                        }
                    }
                }
            }
        }
    }
}
