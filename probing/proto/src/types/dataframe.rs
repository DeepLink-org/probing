use serde::{Deserialize, Serialize};

use super::Ele;
use super::Seq;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Clone)]
pub struct DataFrame {
    pub names: Vec<String>,
    pub cols: Vec<Seq>,
    pub size: u64,
}

impl DataFrame {
    pub fn new(names: Vec<String>, columns: Vec<Seq>) -> Self {
        DataFrame {
            names,
            cols: columns,
            size: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.row_count()
    }

    /// Row count using the longest column (handles ragged columns safely).
    pub fn row_count(&self) -> usize {
        self.cols.iter().map(|c| c.len()).max().unwrap_or(0)
    }

    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    pub fn scalar_f64(&self, col: &str, row: usize) -> Option<f64> {
        let ci = self.col_index(col)?;
        let column = self.cols.get(ci)?;
        if row >= column.len() {
            return None;
        }
        ele_f64(&column.get(row))
    }

    pub fn scalar_i64(&self, col: &str, row: usize) -> Option<i64> {
        let ci = self.col_index(col)?;
        let column = self.cols.get(ci)?;
        if row >= column.len() {
            return None;
        }
        ele_i64(&column.get(row))
    }

    pub fn scalar_boolish(&self, col: &str, row: usize) -> bool {
        let ci = match self.col_index(col) {
            Some(ci) => ci,
            None => return false,
        };
        let column = match self.cols.get(ci) {
            Some(column) => column,
            None => return false,
        };
        if row >= column.len() {
            return false;
        }
        ele_boolish(&column.get(row))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    pub fn iter(&'_ self) -> DataFrameIterator<'_> {
        DataFrameIterator {
            df: self,
            current: 0,
        }
    }
}

fn ele_f64(ele: &Ele) -> Option<f64> {
    match ele {
        Ele::F64(x) => Some(*x),
        Ele::F32(x) => Some(*x as f64),
        Ele::I64(x) => Some(*x as f64),
        Ele::I32(x) => Some(*x as f64),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn ele_i64(ele: &Ele) -> Option<i64> {
    match ele {
        Ele::I64(x) => Some(*x),
        Ele::I32(x) => Some(*x as i64),
        Ele::F64(x) => Some(*x as i64),
        Ele::Text(s) => s.parse().ok(),
        _ => None,
    }
}

fn ele_boolish(ele: &Ele) -> bool {
    match ele {
        Ele::BOOL(x) => *x,
        Ele::I64(x) => *x != 0,
        Ele::I32(x) => *x != 0,
        Ele::Text(s) => matches!(s.as_str(), "1" | "true" | "True"),
        _ => false,
    }
}

pub struct DataFrameIterator<'a> {
    df: &'a DataFrame,
    current: usize,
}

impl Iterator for DataFrameIterator<'_> {
    type Item = Vec<Ele>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.df.len() {
            None
        } else {
            let mut row = vec![];
            for i in 0..self.df.cols.len() {
                row.push(self.df.cols[i].get(self.current));
            }
            self.current += 1;
            Some(row)
        }
    }
}
