use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, SecondsFormat, Utc};
use tea_protocol::ProtocolTimestamp;

use crate::{KernelError, KernelErrorCode};

/// Boxed deadline future returned by a kernel clock.
pub type KernelDeadlineFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Replaceable source of canonical time and deterministic deadlines.
pub trait KernelClock: std::fmt::Debug + Send + Sync {
    /// Returns the current caller-visible protocol timestamp.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the source cannot produce canonical time.
    fn now(&self) -> Result<ProtocolTimestamp, KernelError>;

    /// Completes when the supplied canonical deadline is reached.
    fn sleep_until(&self, deadline: ProtocolTimestamp) -> KernelDeadlineFuture<'_>;
}

/// Production clock backed by system UTC and Tokio deadlines.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioKernelClock;

impl KernelClock for TokioKernelClock {
    fn now(&self) -> Result<ProtocolTimestamp, KernelError> {
        canonical_system_time(SystemTime::now())
    }

    fn sleep_until(&self, deadline: ProtocolTimestamp) -> KernelDeadlineFuture<'_> {
        let duration = self
            .now()
            .ok()
            .and_then(|now| (deadline.as_utc() - now.as_utc()).to_std().ok())
            .unwrap_or(Duration::ZERO);
        Box::pin(tokio::time::sleep(duration))
    }
}

fn canonical_system_time(time: SystemTime) -> Result<ProtocolTimestamp, KernelError> {
    let value = DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Millis, true);
    value.parse().map_err(|_| {
        KernelError::new(
            KernelErrorCode::ClockFailure,
            "system clock did not produce a canonical timestamp",
        )
    })
}
