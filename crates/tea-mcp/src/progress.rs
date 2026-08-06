use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};

use rmcp::model::{NumberOrString, ProgressNotificationParam, ProgressToken};
use tea_tools::ToolProgress;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{McpError, McpErrorCode};

const DEFAULT_PROGRESS_MESSAGE: &str = "MCP tool progress";
const MAX_PROGRESS_TOKEN_BYTES: usize = 256;
const MAX_PROGRESS_MESSAGE_BYTES: usize = 4_096;

#[derive(Debug, Clone)]
pub(crate) struct ProgressRouter {
    state: Arc<Mutex<ProgressRouterState>>,
    max_entries: usize,
    max_pending: usize,
}

impl ProgressRouter {
    pub(crate) fn new(max_entries: usize, max_pending: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProgressRouterState::default())),
            max_entries,
            max_pending,
        }
    }

    pub(crate) fn subscribe(
        &self,
        token: ProgressToken,
        max_events: usize,
    ) -> Result<ProgressInbox, McpError> {
        let mut state = self.lock();
        if state.entries.len() >= self.max_entries
            || state.entries.contains_key(&token)
            || state.retired.contains(&token)
        {
            return Err(McpError::new(McpErrorCode::Transport));
        }
        let (sender, receiver) = mpsc::channel(max_events);
        let overflow = CancellationToken::new();
        let pending = state.pending.remove(&token).unwrap_or_default();
        state.pending_count = state.pending_count.saturating_sub(pending.len());
        let pending_overflow = pending.len() > max_events;
        let accepted = pending.len().min(max_events);
        for notification in pending.into_iter().take(max_events) {
            sender
                .try_send(notification)
                .map_err(|_| McpError::new(McpErrorCode::OutputBound))?;
        }
        if pending_overflow {
            overflow.cancel();
        }
        state.entries.insert(
            token.clone(),
            ProgressEntry {
                sender,
                overflow: overflow.clone(),
                accepted,
                max_events,
            },
        );
        Ok(ProgressInbox {
            token,
            receiver,
            overflow,
            open: true,
        })
    }

    pub(crate) fn route(&self, mut notification: ProgressNotificationParam) {
        if !bounded_token(&notification.progress_token) {
            return;
        }
        if notification.message.as_ref().is_some_and(|message| {
            message.is_empty()
                || message.len() > MAX_PROGRESS_MESSAGE_BYTES
                || message.contains('\0')
        }) {
            notification.message = Some(DEFAULT_PROGRESS_MESSAGE.to_owned());
        }
        notification.meta = None;
        let token = notification.progress_token.clone();
        let mut state = self.lock();
        if state.retired.contains(&token) {
            return;
        }
        let Some(entry) = state.entries.get_mut(&token) else {
            if state.pending_count >= self.max_pending {
                return;
            }
            if !state.pending.contains_key(&token) && state.pending.len() >= self.max_entries {
                return;
            }
            state
                .pending
                .entry(token)
                .or_default()
                .push_back(notification);
            state.pending_count = state.pending_count.saturating_add(1);
            return;
        };
        if entry.accepted >= entry.max_events {
            entry.overflow.cancel();
            return;
        }
        entry.accepted = entry.accepted.saturating_add(1);
        match entry.sender.try_send(notification) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => entry.overflow.cancel(),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                state.entries.remove(&token);
            }
        }
    }

    pub(crate) fn unsubscribe(&self, token: &ProgressToken) {
        let mut state = self.lock();
        state.entries.remove(token);
        if let Some(pending) = state.pending.remove(token) {
            state.pending_count = state.pending_count.saturating_sub(pending.len());
        }
        if state.retired.insert(token.clone()) {
            state.retired_order.push_back(token.clone());
        }
        while state.retired_order.len() > self.max_pending {
            if let Some(expired) = state.retired_order.pop_front() {
                state.retired.remove(&expired);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, ProgressRouterState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Default)]
struct ProgressRouterState {
    entries: HashMap<ProgressToken, ProgressEntry>,
    pending: HashMap<ProgressToken, VecDeque<ProgressNotificationParam>>,
    pending_count: usize,
    retired: HashSet<ProgressToken>,
    retired_order: VecDeque<ProgressToken>,
}

#[derive(Debug)]
struct ProgressEntry {
    sender: mpsc::Sender<ProgressNotificationParam>,
    overflow: CancellationToken,
    accepted: usize,
    max_events: usize,
}

#[derive(Debug)]
pub(crate) struct ProgressInbox {
    pub(crate) token: ProgressToken,
    pub(crate) receiver: mpsc::Receiver<ProgressNotificationParam>,
    pub(crate) overflow: CancellationToken,
    pub(crate) open: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ProgressMapper {
    previous: Option<f64>,
}

impl ProgressMapper {
    pub(crate) fn map(
        &mut self,
        notification: ProgressNotificationParam,
    ) -> Result<ToolProgress, McpError> {
        let progress = notification.progress;
        let total = notification.total;
        if !progress.is_finite()
            || progress.is_sign_negative()
            || self.previous.is_some_and(|previous| progress < previous)
            || total.is_some_and(|total| {
                !total.is_finite() || total.is_sign_negative() || progress > total
            })
            || progress > MAX_EXACT_PROGRESS_UNITS
            || total.is_some_and(|total| total > MAX_EXACT_PROGRESS_UNITS)
        {
            return Err(McpError::new(McpErrorCode::Protocol));
        }
        self.previous = Some(progress);
        let message = notification
            .message
            .filter(|message| {
                !message.is_empty()
                    && message.len() <= MAX_PROGRESS_MESSAGE_BYTES
                    && !message.contains('\0')
            })
            .unwrap_or_else(|| DEFAULT_PROGRESS_MESSAGE.to_owned());
        ToolProgress::new(message, progress_units(progress), total.map(total_units))
            .map_err(|_| McpError::new(McpErrorCode::Protocol))
    }
}

const MAX_EXACT_PROGRESS_UNITS: f64 = 9_007_199_254_740_991.0;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn progress_units(value: f64) -> u64 {
    value.floor() as u64
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn total_units(value: f64) -> u64 {
    value.ceil() as u64
}

fn bounded_token(token: &ProgressToken) -> bool {
    match &token.0 {
        NumberOrString::Number(_) => true,
        NumberOrString::String(value) => value.len() <= MAX_PROGRESS_TOKEN_BYTES,
    }
}
