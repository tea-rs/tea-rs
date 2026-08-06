use std::collections::VecDeque;
use std::sync::Mutex;

use tea_protocol::{CanonicalMessage, CommandText, MessageRole};

use crate::{KernelError, KernelErrorCode};

/// Thread-safe bounded steering and follow-up queue applied between turns.
#[derive(Debug)]
pub struct KernelInputQueue {
    max_messages: usize,
    max_steering_bytes: usize,
    state: Mutex<QueueState>,
}

#[derive(Debug, Clone, Default)]
struct QueueState {
    follow_ups: VecDeque<CanonicalMessage>,
    steering: Vec<CommandText>,
    steering_bytes: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct QueueSnapshot {
    pub(crate) follow_ups: Vec<CanonicalMessage>,
    pub(crate) steering: Vec<CommandText>,
}

impl KernelInputQueue {
    /// Creates an empty bounded queue.
    ///
    /// # Errors
    ///
    /// Returns an error when either limit is zero or unsupported.
    pub fn new(max_messages: usize, max_steering_bytes: usize) -> Result<Self, KernelError> {
        if max_messages == 0
            || max_messages > 1024
            || max_steering_bytes == 0
            || max_steering_bytes > 1024 * 1024
        {
            return Err(KernelError::new(
                KernelErrorCode::InvalidRequest,
                "kernel input queue limits are invalid",
            ));
        }
        Ok(Self {
            max_messages,
            max_steering_bytes,
            state: Mutex::new(QueueState::default()),
        })
    }

    /// Appends one canonical user follow-up in FIFO order.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-user message, queue overflow, or poisoned state.
    pub fn enqueue_follow_up(&self, message: CanonicalMessage) -> Result<(), KernelError> {
        if message.role() != MessageRole::User {
            return Err(KernelError::new(
                KernelErrorCode::InvalidRequest,
                "follow-up queue accepts only user messages",
            ));
        }
        let mut state = self.lock()?;
        if state.follow_ups.len() >= self.max_messages {
            return Err(KernelError::new(
                KernelErrorCode::LimitExceeded,
                "follow-up queue is full",
            ));
        }
        state.follow_ups.push_back(message);
        Ok(())
    }

    /// Appends bounded steering text for coalescing at the next turn boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when accumulated steering exceeds the byte or message limit.
    pub fn enqueue_steering(&self, text: CommandText) -> Result<(), KernelError> {
        let mut state = self.lock()?;
        let bytes = state
            .steering_bytes
            .checked_add(text.as_str().len())
            .and_then(|value| value.checked_add(1))
            .ok_or_else(queue_overflow)?;
        if state.steering.len() >= self.max_messages || bytes > self.max_steering_bytes {
            return Err(queue_overflow());
        }
        state.steering.push(text);
        state.steering_bytes = bytes;
        Ok(())
    }

    /// Returns accepted follow-up and steering counts.
    ///
    /// # Errors
    ///
    /// Returns an error when synchronized state is poisoned.
    pub fn lengths(&self) -> Result<(usize, usize), KernelError> {
        let state = self.lock()?;
        Ok((state.follow_ups.len(), state.steering.len()))
    }

    pub(crate) fn snapshot(&self) -> Result<QueueSnapshot, KernelError> {
        let state = self.lock()?;
        Ok(QueueSnapshot {
            follow_ups: state.follow_ups.iter().cloned().collect(),
            steering: state.steering.clone(),
        })
    }

    pub(crate) fn acknowledge(&self, snapshot: &QueueSnapshot) -> Result<(), KernelError> {
        let mut state = self.lock()?;
        for expected in &snapshot.follow_ups {
            if state.follow_ups.front() != Some(expected) {
                return Err(KernelError::new(
                    KernelErrorCode::InvalidState,
                    "follow-up queue changed before acknowledgement",
                ));
            }
            state.follow_ups.pop_front();
        }
        if state.steering.get(..snapshot.steering.len()) != Some(snapshot.steering.as_slice()) {
            return Err(KernelError::new(
                KernelErrorCode::InvalidState,
                "steering queue changed before acknowledgement",
            ));
        }
        state.steering.drain(..snapshot.steering.len());
        state.steering_bytes = state
            .steering
            .iter()
            .map(|text| text.as_str().len() + 1)
            .sum();
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, QueueState>, KernelError> {
        self.state.lock().map_err(|_| {
            KernelError::new(
                KernelErrorCode::InvalidState,
                "kernel input queue lock is poisoned",
            )
        })
    }
}

fn queue_overflow() -> KernelError {
    KernelError::new(
        KernelErrorCode::LimitExceeded,
        "steering queue limit was reached",
    )
}
