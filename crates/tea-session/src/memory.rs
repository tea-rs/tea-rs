use std::collections::BTreeMap;
use std::sync::RwLock;

use tea_policy::{ActorId, PolicyGrant};
use tea_protocol::SessionId;

use crate::{
    AppendOutcome, AppendTransaction, GrantJournalEntry, SessionStore, SessionStoreError,
    SessionStoreErrorCode, SessionStoreFuture, StoredSession,
};

/// In-memory semantic reference implementation of [`SessionStore`].
///
/// All append validation and materialization is shared with durable stores via
/// [`crate::apply_transaction`].
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    sessions: RwLock<BTreeMap<SessionId, StoredSession>>,
    names: RwLock<BTreeMap<SessionId, crate::SessionName>>,
}

impl InMemorySessionStore {
    /// Creates an empty reference store.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sessions: RwLock::new(BTreeMap::new()),
            names: RwLock::new(BTreeMap::new()),
        }
    }

    fn append_sync(
        &self,
        transaction: &AppendTransaction,
    ) -> Result<AppendOutcome, SessionStoreError> {
        let mut sessions = self.sessions.write().map_err(|_| {
            SessionStoreError::new(
                SessionStoreErrorCode::StorageUnavailable,
                "in-memory session store lock is poisoned",
            )
        })?;
        let known_grant_ids: std::collections::HashSet<tea_policy::GrantId> = sessions
            .values()
            .flat_map(|stored| stored.grant_journal.iter())
            .map(GrantJournalEntry::grant_id)
            .collect();
        let existed = sessions.contains_key(&transaction.session_id());
        let stored = sessions.entry(transaction.session_id()).or_default();
        let outcome = crate::apply_transaction_in_place(transaction, stored, existed, |grant_id| {
            known_grant_ids.contains(&grant_id)
        });
        if outcome.is_err() && !existed {
            sessions.remove(&transaction.session_id());
        }
        outcome
    }
}

impl crate::SessionCatalog for InMemorySessionStore {
    fn list_sessions(&self) -> SessionStoreFuture<'_, Vec<crate::SessionCatalogEntry>> {
        Box::pin(async move {
            let sessions = self.sessions.read().map_err(|_| {
                SessionStoreError::new(
                    SessionStoreErrorCode::StorageUnavailable,
                    "in-memory session store lock is poisoned",
                )
            })?;
            let names = self.names.read().map_err(|_| {
                SessionStoreError::new(
                    SessionStoreErrorCode::StorageUnavailable,
                    "in-memory session name lock is poisoned",
                )
            })?;
            let mut entries = sessions
                .iter()
                .map(|(session_id, stored)| {
                    crate::catalog::catalog_entry(
                        &stored.snapshot(),
                        names.get(session_id).cloned(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            crate::catalog::sort_catalog(&mut entries);
            Ok(entries)
        })
    }

    fn set_session_name(
        &self,
        session_id: SessionId,
        name: Option<crate::SessionName>,
    ) -> SessionStoreFuture<'_, ()> {
        Box::pin(async move {
            if !self
                .sessions
                .read()
                .map_err(|_| {
                    SessionStoreError::new(
                        SessionStoreErrorCode::StorageUnavailable,
                        "in-memory session store lock is poisoned",
                    )
                })?
                .contains_key(&session_id)
            {
                return Err(SessionStoreError::new(
                    SessionStoreErrorCode::SessionNotFound,
                    "session does not exist",
                ));
            }
            let mut names = self.names.write().map_err(|_| {
                SessionStoreError::new(
                    SessionStoreErrorCode::StorageUnavailable,
                    "in-memory session name lock is poisoned",
                )
            })?;
            if let Some(name) = name {
                names.insert(session_id, name);
            } else {
                names.remove(&session_id);
            }
            Ok(())
        })
    }

    fn session_name(
        &self,
        session_id: SessionId,
    ) -> SessionStoreFuture<'_, Option<crate::SessionName>> {
        Box::pin(async move {
            if !self
                .sessions
                .read()
                .map_err(|_| {
                    SessionStoreError::new(
                        SessionStoreErrorCode::StorageUnavailable,
                        "in-memory session store lock is poisoned",
                    )
                })?
                .contains_key(&session_id)
            {
                return Err(SessionStoreError::new(
                    SessionStoreErrorCode::SessionNotFound,
                    "session does not exist",
                ));
            }
            Ok(self
                .names
                .read()
                .map_err(|_| {
                    SessionStoreError::new(
                        SessionStoreErrorCode::StorageUnavailable,
                        "in-memory session name lock is poisoned",
                    )
                })?
                .get(&session_id)
                .cloned())
        })
    }
}

impl SessionStore for InMemorySessionStore {
    fn load(&self, session_id: SessionId) -> SessionStoreFuture<'_, crate::SessionSnapshot> {
        Box::pin(async move {
            self.sessions
                .read()
                .map_err(|_| {
                    SessionStoreError::new(
                        SessionStoreErrorCode::StorageUnavailable,
                        "in-memory session store lock is poisoned",
                    )
                })?
                .get(&session_id)
                .map(StoredSession::snapshot)
                .ok_or_else(|| {
                    SessionStoreError::new(
                        SessionStoreErrorCode::SessionNotFound,
                        "session does not exist",
                    )
                })
        })
    }

    fn append(&self, transaction: AppendTransaction) -> SessionStoreFuture<'_, AppendOutcome> {
        Box::pin(async move { self.append_sync(&transaction) })
    }

    fn active_grants_for_actor(
        &self,
        actor_id: ActorId,
    ) -> SessionStoreFuture<'_, Vec<PolicyGrant>> {
        Box::pin(async move {
            let sessions = self.sessions.read().map_err(|_| {
                SessionStoreError::new(
                    SessionStoreErrorCode::StorageUnavailable,
                    "in-memory session store lock is poisoned",
                )
            })?;
            Ok(crate::active_grants_for_actor(sessions.values(), &actor_id))
        })
    }
}
