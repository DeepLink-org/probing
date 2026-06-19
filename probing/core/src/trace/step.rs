use std::cell::RefCell;

/// Snapshot of the training step coordinate system (scheme 2: local step buckets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepSnapshot {
    pub local_step: u64,
    pub global_step: u64,
    pub bucket_size: u64,
    pub rank: i64,
    pub world_size: i64,
}

#[derive(Debug, Clone)]
struct StepContext {
    local_step: u64,
    bucket_size: u64,
    rank: i64,
    world_size: i64,
}

impl Default for StepContext {
    fn default() -> Self {
        Self {
            local_step: 0,
            bucket_size: read_bucket_size(),
            rank: read_rank(),
            world_size: read_world_size(),
        }
    }
}

impl StepContext {
    fn snapshot(&self) -> StepSnapshot {
        StepSnapshot {
            local_step: self.local_step,
            global_step: global_step_for(self.local_step, self.bucket_size),
            bucket_size: self.bucket_size,
            rank: self.rank,
            world_size: self.world_size,
        }
    }

    fn sync_local_step(&mut self, step: u64) -> StepSnapshot {
        self.local_step = step;
        self.snapshot()
    }

    fn advance_local_step(&mut self) -> StepSnapshot {
        self.local_step = self.local_step.saturating_add(1);
        self.snapshot()
    }

    fn set_bucket_size(&mut self, bucket: u64) {
        self.bucket_size = bucket.max(1);
    }
}

thread_local! {
    static STEP_CTX: RefCell<StepContext> = RefCell::new(StepContext::default());
}

fn global_step_for(local_step: u64, bucket_size: u64) -> u64 {
    let bucket = bucket_size.max(1);
    local_step / bucket
}

fn read_env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn read_env_i64(key: &str) -> Option<i64> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn read_bucket_size() -> u64 {
    read_env_u64("PROBING_GLOBAL_STEP_BUCKET")
        .or_else(|| read_env_u64("PROBING_STEP_BUCKET"))
        .unwrap_or(1)
        .max(1)
}

fn read_rank() -> i64 {
    read_env_i64("RANK").unwrap_or(0)
}

fn read_world_size() -> i64 {
    read_env_i64("WORLD_SIZE").unwrap_or(1)
}

fn with_ctx<R>(f: impl FnOnce(&mut StepContext) -> R) -> R {
    STEP_CTX.with(|ctx| f(&mut ctx.borrow_mut()))
}

pub fn step_snapshot() -> StepSnapshot {
    with_ctx(|ctx| ctx.snapshot())
}

pub fn sync_local_step(step: u64) -> StepSnapshot {
    with_ctx(|ctx| ctx.sync_local_step(step))
}

pub fn advance_local_step() -> StepSnapshot {
    with_ctx(|ctx| ctx.advance_local_step())
}

pub fn set_step_bucket_size(bucket: u64) {
    with_ctx(|ctx| ctx.set_bucket_size(bucket));
}

pub fn current_local_step() -> u64 {
    step_snapshot().local_step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_step_uses_bucket_size() {
        assert_eq!(global_step_for(0, 1), 0);
        assert_eq!(global_step_for(9, 10), 0);
        assert_eq!(global_step_for(10, 10), 1);
        assert_eq!(global_step_for(42, 10), 4);
    }

    #[test]
    fn advance_and_sync_local_step() {
        let _ = sync_local_step(0);
        assert_eq!(step_snapshot().local_step, 0);
        let snap = advance_local_step();
        assert_eq!(snap.local_step, 1);
        assert_eq!(snap.global_step, 1);
        let snap = sync_local_step(99);
        assert_eq!(snap.local_step, 99);
        assert_eq!(snap.global_step, 99);
    }
}
