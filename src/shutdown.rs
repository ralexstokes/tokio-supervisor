use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownMode {
    Cooperative,
    CooperativeThenAbort,
    Abort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownPolicy {
    pub grace: Duration,
    pub mode: ShutdownMode,
}

impl ShutdownPolicy {
    pub fn new(grace: Duration, mode: ShutdownMode) -> Self {
        Self { grace, mode }
    }

    pub fn cooperative(grace: Duration) -> Self {
        Self::new(grace, ShutdownMode::Cooperative)
    }

    pub fn cooperative_then_abort(grace: Duration) -> Self {
        Self::new(grace, ShutdownMode::CooperativeThenAbort)
    }

    pub fn abort() -> Self {
        Self::new(Duration::ZERO, ShutdownMode::Abort)
    }
}

impl Default for ShutdownPolicy {
    fn default() -> Self {
        Self::cooperative_then_abort(Duration::from_secs(5))
    }
}
