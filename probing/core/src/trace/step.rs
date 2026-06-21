use std::cell::RefCell;

/// Training step coordinates.
///
/// * ``micro_step`` — finest counter (advanced each ``train.step`` / ``probing.step()``).
/// * ``local_step = micro_step / micro_batches`` — per-rank training step.
/// * ``global_step = local_step`` — cluster training step (same value when ranks align).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepSnapshot {
    pub micro_step: u64,
    pub local_step: u64,
    pub global_step: u64,
    pub micro_batches: u64,
    pub rank: i64,
    pub world_size: i64,
}

#[derive(Debug, Clone)]
struct StepContext {
    micro_step: u64,
    micro_batches: u64,
    rank: i64,
    world_size: i64,
}

impl Default for StepContext {
    fn default() -> Self {
        Self {
            micro_step: 0,
            micro_batches: read_micro_batches(),
            rank: read_rank(),
            world_size: read_world_size(),
        }
    }
}

impl StepContext {
    fn snapshot(&self) -> StepSnapshot {
        let local = training_step_for(self.micro_step, self.micro_batches);
        StepSnapshot {
            micro_step: self.micro_step,
            local_step: local,
            global_step: local,
            micro_batches: self.micro_batches,
            rank: self.rank,
            world_size: self.world_size,
        }
    }

    fn sync_micro_step(&mut self, step: u64) -> StepSnapshot {
        self.micro_step = step;
        self.snapshot()
    }

    fn advance_micro_step(&mut self) -> StepSnapshot {
        self.micro_step = self.micro_step.saturating_add(1);
        self.snapshot()
    }

    fn set_micro_batches(&mut self, micro_batches: u64) {
        self.micro_batches = micro_batches.max(1);
    }
}

thread_local! {
    static STEP_CTX: RefCell<StepContext> = RefCell::new(StepContext::default());
}

fn training_step_for(micro_step: u64, micro_batches: u64) -> u64 {
    micro_step / micro_batches.max(1)
}

fn read_env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn read_env_i64(key: &str) -> Option<i64> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

fn read_micro_batches() -> u64 {
    read_env_u64("PROBING_MICRO_BATCHES")
        .or_else(|| read_env_u64("PROBING_GLOBAL_STEP_BUCKET"))
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

pub fn sync_micro_step(step: u64) -> StepSnapshot {
    with_ctx(|ctx| ctx.sync_micro_step(step))
}

pub fn advance_micro_step() -> StepSnapshot {
    with_ctx(|ctx| ctx.advance_micro_step())
}

pub fn set_micro_batches(micro_batches: u64) {
    with_ctx(|ctx| ctx.set_micro_batches(micro_batches));
}

pub fn current_micro_step() -> u64 {
    step_snapshot().micro_step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_step_uses_micro_batches() {
        assert_eq!(training_step_for(0, 1), 0);
        assert_eq!(training_step_for(9, 10), 0);
        assert_eq!(training_step_for(10, 10), 1);
        assert_eq!(training_step_for(42, 10), 4);
    }

    #[test]
    fn global_step_equals_local_step() {
        let _ = sync_micro_step(0);
        let snap = advance_micro_step();
        assert_eq!(snap.micro_step, 1);
        assert_eq!(snap.local_step, 1);
        assert_eq!(snap.global_step, 1);
        let snap = sync_micro_step(99);
        assert_eq!(snap.micro_step, 99);
        assert_eq!(snap.local_step, 99);
        assert_eq!(snap.global_step, 99);
    }

    #[test]
    fn micro_batches_groups_training_steps() {
        set_micro_batches(10);
        let _ = sync_micro_step(0);
        let snap = sync_micro_step(15);
        assert_eq!(snap.micro_step, 15);
        assert_eq!(snap.local_step, 1);
        assert_eq!(snap.global_step, 1);
        set_micro_batches(1);
    }
}
