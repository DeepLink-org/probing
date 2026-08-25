use probing_proto::prelude::DataFrame;

pub(super) fn tag_dataframe(
    mut dataframe: DataFrame,
    host: &str,
    addr: &str,
    rank: Option<i32>,
) -> DataFrame {
    if dataframe.is_empty() {
        return dataframe;
    }
    probing_core::core::federation::tag_proto_dataframe(&mut dataframe, host, addr, rank);
    dataframe
}

pub(super) fn merge_tagged_dataframes(parts: &[DataFrame]) -> DataFrame {
    probing_proto::types::merge_dataframes(parts)
}

#[cfg(test)]
mod tests {
    use probing_proto::prelude::Seq;

    use super::*;

    #[test]
    fn merge_preserves_probe_tags() {
        let local = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![0])],
                size: 1,
            },
            "host-a",
            "10.0.0.1:8080",
            Some(0),
        );
        let remote = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![1])],
                size: 1,
            },
            "host-b",
            "10.0.0.2:8080",
            Some(1),
        );
        let merged = merge_tagged_dataframes(&[local, remote]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.names.len(), 7);
        let host_col = merged
            .names
            .iter()
            .position(|name| name == "_host")
            .unwrap();
        assert_eq!(merged.cols[host_col].get_str(0).as_deref(), Some("host-a"));
        assert_eq!(merged.cols[host_col].get_str(1).as_deref(), Some("host-b"));
    }

    #[test]
    fn merge_aligns_missing_columns_with_empty_strings() {
        let a = DataFrame {
            names: vec!["x".into(), "extra".into()],
            cols: vec![Seq::SeqI32(vec![1]), Seq::SeqText(vec!["a".into()])],
            size: 1,
        };
        let b = DataFrame {
            names: vec!["x".into()],
            cols: vec![Seq::SeqI32(vec![2])],
            size: 1,
        };
        let merged = merge_tagged_dataframes(&[a, b]);
        assert_eq!(merged.len(), 2);
        assert!(merged.names.contains(&"extra".to_string()));
    }
}
