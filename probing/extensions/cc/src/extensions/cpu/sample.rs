//! Canonical CPU sample types shared by all platform backends.

/// Process-level CPU snapshot (cumulative counters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSample {
    pub cputime_user_ns: u64,
    pub cputime_sys_ns: u64,
    pub rss_bytes: u64,
    pub thread_count: u32,
    pub vol_ctxt: u64,
    pub invol_ctxt: u64,
}

impl ProcessSample {
    pub fn total_cputime_ns(&self) -> u64 {
        self.cputime_user_ns.saturating_add(self.cputime_sys_ns)
    }
}

/// Thread-level CPU snapshot (cumulative counters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSample {
    pub tid: i32,
    pub comm: String,
    pub state: Option<String>,
    pub wchan: Option<String>,
    pub cputime_user_ns: u64,
    pub cputime_sys_ns: u64,
}

impl ThreadSample {
    pub fn total_cputime_ns(&self) -> u64 {
        self.cputime_user_ns.saturating_add(self.cputime_sys_ns)
    }
}
