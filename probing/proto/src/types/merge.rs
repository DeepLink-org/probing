use super::{DataFrame, Ele, Seq};

/// Append rows from `other` into `base`, aligning columns by name (missing values become `Nil`).
pub fn append_dataframe(base: &mut DataFrame, other: &DataFrame) {
    if other.is_empty() {
        return;
    }
    if base.is_empty() {
        *base = other.clone();
        return;
    }

    let other_rows = other.len();
    for name in &other.names {
        if !base.names.contains(name) {
            base.names.push(name.clone());
            base.cols
                .push(Seq::SeqText(vec![String::new(); base.len()]));
        }
    }
    for (col_idx, name) in base.names.clone().iter().enumerate() {
        let src_idx = other.names.iter().position(|n| n == name);
        for row in 0..other_rows {
            let ele = src_idx
                .and_then(|i| other.cols.get(i).map(|c| c.get(row)))
                .unwrap_or(Ele::Nil);
            if let Some(col) = base.cols.get_mut(col_idx) {
                let _ = col.append(ele);
            }
        }
    }
    base.size = base.len() as u64;
}

/// Merge non-empty DataFrames with column alignment (preserves row order of `parts`).
pub fn merge_dataframes(parts: &[DataFrame]) -> DataFrame {
    let mut out = DataFrame::default();
    for df in parts {
        if df.is_empty() {
            continue;
        }
        if out.is_empty() {
            out = df.clone();
            continue;
        }
        append_dataframe(&mut out, df);
    }
    out.size = out.len() as u64;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{Ele, Seq};

    #[test]
    fn merge_aligns_columns() {
        let a = DataFrame {
            names: vec!["rank".into()],
            cols: vec![Seq::SeqI32(vec![0])],
            size: 1,
        };
        let b = DataFrame {
            names: vec!["rank".into(), "extra".into()],
            cols: vec![Seq::SeqI32(vec![1]), Seq::SeqText(vec!["x".into()])],
            size: 1,
        };
        let merged = merge_dataframes(&[a, b]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.names, vec!["rank", "extra"]);
        assert_eq!(merged.cols[0].get(1), Ele::I32(1));
    }
}
