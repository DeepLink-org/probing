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
                    tr { class: "bg-gray-50 border-b border-gray-200 sticky top-0 z-10",
                        for (col_idx, header) in headers.iter().enumerate() {
                            th {
                                class: format!("px-4 py-2 text-left font-semibold text-gray-700 border-r border-gray-200 bg-gray-50 {} {}", if col_idx == 0 { "sticky left-0 z-10" } else { "" }, ""),
                                {header.clone()}
                            }
                        }
                    }
                }

                tbody {
                    for (row_idx, row) in data.iter().enumerate() {
                        tr {
                            class: if row_idx % 2 == 0 { "bg-white hover:bg-gray-50" } else { "bg-gray-50 hover:bg-gray-100" },
                            onclick: move |_| {
                                if let Some(cb) = on_row_click {
                                    cb.call(row_idx);
                                }
                            },
                            for (cell_idx, cell) in row.iter().enumerate() {
                                td {
                                    class: format!("px-4 py-2 text-gray-700 border-r border-gray-200 {} {}", if cell_idx == 0 { "sticky left-0 z-[1]" } else { "" }, if cell_idx == 0 && row_idx % 2 == 0 { "bg-white" } else if cell_idx == 0 { "bg-gray-50" } else { "" }),
                                    {cell.clone()}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
