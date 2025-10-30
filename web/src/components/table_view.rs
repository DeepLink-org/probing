use dioxus::prelude::*;

#[component]
pub fn TableView(headers: Vec<String>, data: Vec<Vec<String>>) -> Element {
    rsx! {
        div {
            class: "table-container",
            style: "overflow-x: auto; border: 1px solid #e5e7eb; border-radius: 8px;",
            
            table {
                class: "table",
                style: "width: 100%; border-collapse: collapse; table-layout: auto;",
                
                // Table header
                thead {
                    tr {
                        style: "background: #f9fafb; border-bottom: 1px solid #e5e7eb;",
                        for header in headers {
                            th {
                                style: "padding: 12px 16px; text-align: left; font-weight: 600; color: #374151; border-right: 1px solid #e5e7eb;",
                                {header}
                            }
                        }
                    }
                }
                
                // Table body
                tbody {
                    for (row_idx, row) in data.iter().enumerate() {
                        tr {
                            style: if row_idx % 2 == 0 { "background: white;" } else { "background: #f9fafb;" },
                            for cell in row {
                                td {
                                    style: "padding: 12px 16px; border-right: 1px solid #e5e7eb; color: #374151;",
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
