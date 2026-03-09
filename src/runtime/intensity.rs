use std::{collections::VecDeque, time::Duration};

use tokio::time::Instant;

use crate::restart::{BackoffPolicy, RestartIntensity};

pub(crate) struct RestartTracker {
    intensity: RestartIntensity,
    times: VecDeque<Instant>,
}

impl RestartTracker {
    pub(crate) fn new(intensity: RestartIntensity) -> Self {
        Self {
            intensity,
            times: VecDeque::new(),
        }
    }

    pub(crate) fn record(&mut self, now: Instant) {
        while let Some(front) = self.times.front() {
            if now.duration_since(*front) > self.intensity.within {
                self.times.pop_front();
            } else {
                break;
            }
        }
        self.times.push_back(now);
    }

    pub(crate) fn exceeded(&self) -> bool {
        self.times.len() > self.intensity.max_restarts
    }

    pub(crate) fn backoff(&self) -> Duration {
        match self.intensity.backoff {
            BackoffPolicy::None => Duration::ZERO,
            BackoffPolicy::Fixed(delay) => delay,
            BackoffPolicy::Exponential { base, factor, max } => {
                let mut delay = base;
                let steps = self.times.len().saturating_sub(1);
                for _ in 0..steps {
                    delay = delay.saturating_mul(factor);
                    if delay >= max {
                        return max;
                    }
                }
                delay.min(max)
            }
        }
    }
}
