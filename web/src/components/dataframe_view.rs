use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};
use crate::components::table_view::TableView;

#[component]
pub fn DataFrameView(df: DataFrame, #[props(optional)] on_row_click: Option<EventHandler<usize>>) -> Element {
    let headers = use_memo(move || df.names.clone());
    
    let data = use_memo(move || {
        let nrows = df.cols.iter().map(|x| x.len()).max().unwrap_or(0);
        (0..nrows)
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
            .collect::<Vec<Vec<String>>>()
    });
    
    rsx! { TableView { headers: headers.read().clone(), data: data.read().clone(), on_row_click } }
}
