use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};
use crate::components::table_view::TableView;

#[component]
pub fn DataFrameView(df: DataFrame) -> Element {
    let nrows = df.cols.iter().map(|x| x.len()).max().unwrap_or(0);
    
    // Build headers from column names
    let headers = df.names.clone();
    
    // Build data rows
    let data: Vec<Vec<String>> = (0..nrows)
        .map(|i| {
            df.cols
                .iter()
                .map(move |col| {
                    match col.get(i) {
                        Ele::Nil => "nil".to_string(),
                        Ele::BOOL(x) => x.to_string(),
                        Ele::I32(x) => x.to_string(),
                        Ele::I64(x) => x.to_string(),
                        Ele::F32(x) => x.to_string(),
                        Ele::F64(x) => x.to_string(),
                        Ele::Text(x) => x.to_string(),
                        Ele::Url(x) => x.to_string(),
                        Ele::DataTime(x) => x.to_string(),
                    }
                })
                .collect()
        })
        .collect();
    
    rsx! { TableView { headers: headers, data: data } }
}
