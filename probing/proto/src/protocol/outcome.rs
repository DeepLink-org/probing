use serde::{Deserialize, Serialize};

/// Completeness of a query or distributed diagnostic result.
///
/// This is part of the result contract: callers must not infer completeness
/// from HTTP status codes, logs, or request-local side channels.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryQuality {
    #[serde(default)]
    pub nodes_succeeded: usize,
    #[serde(default)]
    pub nodes_failed: Vec<String>,
    #[serde(default)]
    pub peer_batches_dropped: usize,
    #[serde(default)]
    pub partial: bool,
}

impl QueryQuality {
    pub fn complete_node() -> Self {
        Self {
            nodes_succeeded: 1,
            ..Self::default()
        }
    }

    pub fn is_partial(&self) -> bool {
        self.partial || !self.nodes_failed.is_empty() || self.peer_batches_dropped > 0
    }

    pub fn absorb(&mut self, child: Self) {
        self.nodes_succeeded += child.nodes_succeeded;
        self.nodes_failed.extend(child.nodes_failed);
        self.peer_batches_dropped += child.peer_batches_dropped;
        self.partial |= child.partial;
    }
}

/// A value and the quality/completeness of the evidence used to produce it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryOutcome<T> {
    pub data: T,
    #[serde(default)]
    pub quality: QueryQuality,
}

impl<T> QueryOutcome<T> {
    pub fn complete(data: T) -> Self {
        Self {
            data,
            quality: QueryQuality::default(),
        }
    }

    pub fn with_quality(data: T, quality: QueryQuality) -> Self {
        Self { data, quality }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> QueryOutcome<U> {
        QueryOutcome {
            data: map(self.data),
            quality: self.quality,
        }
    }
}

/// Typed metadata carried by the common wire envelope.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<QueryQuality>,
}

impl MessageMeta {
    pub fn from_quality(quality: QueryQuality) -> Option<Self> {
        (quality != QueryQuality::default()).then_some(Self {
            fanout: Some(quality),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_is_partial_for_every_incomplete_signal() {
        assert!(!QueryQuality::default().is_partial());
        assert!(QueryQuality {
            nodes_failed: vec!["rank-1".into()],
            ..QueryQuality::default()
        }
        .is_partial());
        assert!(QueryQuality {
            peer_batches_dropped: 1,
            ..QueryQuality::default()
        }
        .is_partial());
    }

    #[test]
    fn message_meta_omits_complete_quality() {
        assert!(MessageMeta::from_quality(QueryQuality::default()).is_none());
        assert!(MessageMeta::from_quality(QueryQuality::complete_node()).is_some());
    }
}
