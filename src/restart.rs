use std::time::Duration;

use crate::error::BuildError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Restart {
    Permanent,
    #[default]
    Transient,
    Temporary,
}

impl Restart {
    pub(crate) fn should_restart(self, is_failure: bool) -> bool {
        match self {
            Self::Permanent => true,
            Self::Transient => is_failure,
            Self::Temporary => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackoffPolicy {
    #[default]
    None,
    Fixed(Duration),
    Exponential {
        base: Duration,
        factor: u32,
        max: Duration,
    },
    JitteredExponential {
        base: Duration,
        factor: u32,
        max: Duration,
    },
}

impl BackoffPolicy {
    fn validate(self) -> Result<(), BuildError> {
        match self {
            Self::None => Ok(()),
            Self::Fixed(delay) => {
                require_non_zero_duration(delay, "fixed backoff delay must be non-zero")
            }
            Self::Exponential { base, factor, max }
            | Self::JitteredExponential { base, factor, max } => {
                require_non_zero_duration(base, "exponential backoff base must be non-zero")?;
                if factor == 0 {
                    return Err(BuildError::InvalidConfig(
                        "exponential backoff factor must be non-zero",
                    ));
                }
                require_non_zero_duration(max, "exponential backoff max must be non-zero")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartIntensity {
    pub max_restarts: usize,
    pub within: Duration,
    pub backoff: BackoffPolicy,
}

impl Default for RestartIntensity {
    fn default() -> Self {
        Self::new(5, Duration::from_secs(30))
    }
}

impl RestartIntensity {
    pub fn new(max_restarts: usize, within: Duration) -> Self {
        Self {
            max_restarts,
            within,
            backoff: BackoffPolicy::None,
        }
    }

    #[must_use]
    pub fn with_backoff(mut self, backoff: BackoffPolicy) -> Self {
        self.backoff = backoff;
        self
    }

    pub(crate) fn validate(&self) -> Result<(), BuildError> {
        require_non_zero_duration(self.within, "restart intensity window must be non-zero")?;
        self.backoff.validate()
    }
}

fn require_non_zero_duration(duration: Duration, message: &'static str) -> Result<(), BuildError> {
    if duration.is_zero() {
        return Err(BuildError::InvalidConfig(message));
    }

    Ok(())
}
