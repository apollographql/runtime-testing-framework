//! Retry for HTTP requests with exponential backoff and jitter.
use rand::RngExt;
use reqwest::StatusCode;
use std::{fmt::Display, future::Future, time::Duration};
use tokio::time::sleep;
use tracing::warn;

const INITIAL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RETRIES: u32 = 5;
const RATIO: f64 = 1.5;
const JITTER: f64 = 0.25;

#[derive(Debug)]
pub(crate) struct Failure<E> {
    error: E,
    retryable: bool,
}

impl<E> Failure<E> {
    pub(crate) fn from_reqwest(error: E, cause: &reqwest::Error) -> Self {
        let is_transient = !(cause.is_builder() || cause.is_redirect() || cause.is_decode());

        Self {
            error,
            retryable: is_transient,
        }
    }

    pub(crate) fn from_status(error: E, status: StatusCode) -> Self {
        let retryable = status.is_server_error()
            || status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS;

        Self { error, retryable }
    }
}

/// How many times, and how slowly, a failed request is retried.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryConfig {
    max_retries: u32,
    initial_interval: Duration,
    ratio: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: MAX_RETRIES,
            initial_interval: INITIAL_INTERVAL,
            ratio: RATIO,
        }
    }
}

impl RetryConfig {
    fn jittered_delay(&self, retry: u32) -> Duration {
        let factor = rand::rng().random_range(1.0 - JITTER..=1.0 + JITTER);

        self.scaled_delay(retry, factor)
    }

    fn scaled_delay(&self, retry: u32, factor: f64) -> Duration {
        let secs = self.initial_interval.as_secs_f64() * self.ratio.powi(retry as i32) * factor;

        Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX)
    }

    /// Run `op` according to this [RetryConfig]'s parameters.
    ///
    /// Non-retryable errors are returned eagerly and maxing out the number of retries will return
    /// the final error encountered.
    pub(crate) async fn run<T, E, F, Fut>(&self, mut op: F) -> Result<T, E>
    where
        E: Display,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, Failure<E>>>,
    {
        let mut retries = 0;

        loop {
            match op().await {
                Ok(value) => return Ok(value),

                Err(Failure { error, retryable }) if retryable && retries < self.max_retries => {
                    let delay = self.jittered_delay(retries);
                    retries += 1;
                    warn!(
                        retry = retries,
                        max_retries = self.max_retries,
                        ?delay,
                        "request failed, retrying: {error}"
                    );

                    sleep(delay).await;
                }

                Err(Failure { error, .. }) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    impl<E> Failure<E> {
        fn new(error: E, retryable: bool) -> Self {
            Self { error, retryable }
        }
    }

    fn no_delay_cfg(max_retries: u32) -> RetryConfig {
        RetryConfig {
            max_retries,
            initial_interval: Duration::ZERO,
            ratio: 1.5,
        }
    }

    #[test]
    fn scaled_delay_grows_by_ratio_from_initial_interval() {
        let cfg = RetryConfig::default();

        assert_eq!(cfg.scaled_delay(0, 1.0), Duration::from_millis(2000));
        assert_eq!(cfg.scaled_delay(1, 1.0), Duration::from_millis(3000));
        assert_eq!(cfg.scaled_delay(2, 1.0), Duration::from_millis(4500));
    }

    #[tokio::test]
    async fn run_returns_first_success_without_retrying() {
        let calls = AtomicU32::new(0);

        let res: Result<_, String> = no_delay_cfg(3)
            .run(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("done")
            })
            .await;

        assert_eq!(res.unwrap(), "done");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_retries_retryable_failures_until_success() {
        let calls = AtomicU32::new(0);

        let res: Result<_, String> = no_delay_cfg(5)
            .run(|| async {
                match calls.fetch_add(1, Ordering::SeqCst) {
                    0..=2 => Err(Failure::new("transient".to_string(), true)),
                    _ => Ok("done"),
                }
            })
            .await;

        assert_eq!(res.unwrap(), "done");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn run_returns_last_error_once_retries_are_exhausted() {
        let calls = AtomicU32::new(0);

        let res: Result<(), String> = no_delay_cfg(3)
            .run(|| async {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                Err(Failure::new(format!("attempt {n}"), true))
            })
            .await;

        // The initial attempt plus 3 retries
        assert_eq!(res.unwrap_err(), "attempt 3");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn run_does_not_retry_fatal_failures() {
        let calls = AtomicU32::new(0);

        let res: Result<(), String> = no_delay_cfg(5)
            .run(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Failure::new("nope".to_string(), false))
            })
            .await;

        assert_eq!(res.unwrap_err(), "nope");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_with_zero_retries_attempts_once() {
        let calls = AtomicU32::new(0);

        let res: Result<(), String> = no_delay_cfg(0)
            .run(|| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Failure::new("transient".to_string(), true))
            })
            .await;

        assert!(res.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
