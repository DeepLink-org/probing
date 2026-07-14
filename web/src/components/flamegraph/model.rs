use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FlameFrame {
    pub id: usize,
    pub parent: Option<usize>,
    pub name: String,
    pub value: u64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    #[serde(rename = "d")]
    pub depth: usize,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(rename = "modulePath", default)]
    pub module_path: Option<String>,
    /// Training ranks that contributed samples under this frame (distributed only).
    #[serde(default)]
    pub ranks: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FlamegraphPayload {
    pub profile: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(rename = "countName")]
    pub count_name: String,
    #[serde(default)]
    pub metric: Option<String>,
    pub total: u64,
    pub width: f64,
    #[serde(rename = "frameHeight")]
    pub frame_height: f64,
    pub frames: Vec<FlameFrame>,
    #[serde(rename = "emptyMessage", default)]
    pub empty_message: Option<String>,
    /// Samples discarded by the sampler (ring full or cardinality cap). Surfaced
    /// as a warning; 0 / absent for profilers that don't report it.
    #[serde(default)]
    pub dropped: u64,
    /// Number of ranks included in a distributed merge (when present).
    #[serde(rename = "rankCount", default)]
    pub rank_count: Option<usize>,
}
