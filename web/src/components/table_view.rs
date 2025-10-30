use dioxus::prelude::*;
use crate::styles::combinations::*;

#[component]
pub fn TableView(headers: Vec<String>, data: Vec<Vec<String>>) -> Element {
    rsx! {
        div {
            class: TABLE_CONTAINER,

            table {
                class: TABLE,

                thead {
                    tr { class: TABLE_HEADER_ROW,
                        for header in headers {
                            th { class: TABLE_HEADER_CELL, {header} }
                        }
                    }
                }

                tbody {
                    for (row_idx, row) in data.iter().enumerate() {
                        tr { class: if row_idx % 2 == 0 { TABLE_ROW_EVEN } else { TABLE_ROW_ODD },
                            for cell in row {
                                td { class: TABLE_CELL, {cell.clone()} }
                            }
                        }
                    }
                }
            }
        }
    }
}
