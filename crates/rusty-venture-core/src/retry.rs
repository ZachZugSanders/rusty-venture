use std::time::Duration;

/// Defines how a step should be retried after failure.
#[derive(Clone, Debug, Default)]
pub enum RetryStrategy {
    /// Do not retry — fail immediately on first error.
    #[default]
    None,

    /// Retry up to `max_attempts` times with a fixed delay between each.
    Fixed { max_attempts: u32, delay: Duration },

    /// Exponential backoff: each retry doubles the delay, capped at `max_delay`.
    Exponential {
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    },
}

impl RetryStrategy {
    /// Returns the total number of allowed attempts (initial + retries).
    pub fn max_attempts(&self) -> u32 {
        match self {
            RetryStrategy::None => 1,
            RetryStrategy::Fixed { max_attempts, .. } => *max_attempts,
            RetryStrategy::Exponential { max_attempts, .. } => *max_attempts,
        }
    }

    /// Returns the delay to wait before attempt number `attempt` (1-indexed).
    /// Returns `None` if no delay should be applied (first attempt or None strategy).
    pub fn delay_for(&self, attempt: u32) -> Option<Duration> {
        match self {
            RetryStrategy::None => None,
            RetryStrategy::Fixed { delay, .. } => {
                if attempt > 1 {
                    Some(*delay)
                } else {
                    None
                }
            }
            RetryStrategy::Exponential {
                base_delay,
                max_delay,
                ..
            } => {
                if attempt <= 1 {
                    None
                } else {
                    let factor = 2u64.saturating_pow(attempt - 2);
                    let delay = base_delay.saturating_mul(factor as u32);
                    Some(delay.min(*max_delay))
                }
            }
        }
    }
}
