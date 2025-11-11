use serde::{Deserialize, Serialize};

use super::basic::Ele;

/// Time series structure for DTO
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct TimeSeries {
    pub names: Vec<String>,
    pub timestamp: Vec<Ele>,
    pub cols: Vec<Vec<Ele>>,
}

impl TimeSeries {
    pub fn len(&self) -> usize {
        self.timestamp.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&'_ self) -> TimeSeriesIter<'_> {
        TimeSeriesIter {
            timestamp: self.timestamp.iter(),
            cols: self.cols.iter().map(|s| s.iter()).collect(),
        }
    }

    pub fn take(&self, limit: Option<usize>) -> Vec<(Ele, Vec<Ele>)> {
        let iter = self.iter();
        if let Some(limit) = limit {
            iter.take(limit).collect::<Vec<_>>()
        } else {
            iter.collect::<Vec<_>>()
        }
    }
}

pub struct TimeSeriesIter<'a> {
    timestamp: std::slice::Iter<'a, Ele>,
    cols: Vec<std::slice::Iter<'a, Ele>>,
}

impl Iterator for TimeSeriesIter<'_> {
    type Item = (Ele, Vec<Ele>);

    fn next(&mut self) -> Option<Self::Item> {
        let timestamp = self.timestamp.next()?.clone();
        let cols = self
            .cols
            .iter_mut()
            .map(|s| s.next().cloned())
            .collect::<Option<Vec<_>>>()?;
        Some((timestamp, cols))
    }
}
