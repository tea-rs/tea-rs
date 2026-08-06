use std::collections::BTreeMap;
use std::fmt;
use std::mem::size_of;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;

use super::{
    FetchBodyLimits, FetchHttpConfig, FetchProviderError, FetchProviderErrorCode, FetchResult,
};

/// Default lifetime of a normalized fetch cache entry.
pub const DEFAULT_FETCH_CACHE_TTL: Duration = Duration::from_mins(5);
/// Maximum configurable fetch cache entry lifetime.
pub const MAX_FETCH_CACHE_TTL: Duration = Duration::from_hours(1);
/// Default maximum number of normalized fetch cache entries.
pub const DEFAULT_FETCH_CACHE_ENTRIES: usize = 64;
/// Absolute maximum number of normalized fetch cache entries.
pub const MAX_FETCH_CACHE_ENTRIES: usize = 1_024;
/// Default aggregate logical bytes retained by the fetch cache.
pub const DEFAULT_FETCH_CACHE_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Absolute aggregate logical byte limit for the fetch cache.
pub const MAX_FETCH_CACHE_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum logical bytes retained by one fetch cache entry.
pub const DEFAULT_FETCH_CACHE_ENTRY_BYTES: usize = 1024 * 1024;
/// Absolute logical byte limit for one fetch cache entry.
pub const MAX_FETCH_CACHE_ENTRY_BYTES: usize = 4 * 1024 * 1024;

const MAX_FETCH_CACHE_SCOPE_BYTES: usize = 4 * 1024;
const CACHE_ENTRY_OVERHEAD_BYTES: usize = 512;

/// Independently bounded TTL, entry-count, and byte limits for fetch caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchCacheConfig {
    ttl: Duration,
    entries: usize,
    total_bytes: usize,
    entry_bytes: usize,
}

impl FetchCacheConfig {
    /// Creates a bounded cache configuration.
    ///
    /// # Errors
    ///
    /// Rejects zero values, values above absolute limits, or a per-entry byte
    /// limit larger than the aggregate byte limit.
    pub fn new(
        ttl: Duration,
        max_entries: usize,
        max_total_bytes: usize,
        max_entry_bytes: usize,
    ) -> Result<Self, FetchProviderError> {
        if ttl.is_zero()
            || ttl > MAX_FETCH_CACHE_TTL
            || !(1..=MAX_FETCH_CACHE_ENTRIES).contains(&max_entries)
            || !(1..=MAX_FETCH_CACHE_TOTAL_BYTES).contains(&max_total_bytes)
            || !(1..=MAX_FETCH_CACHE_ENTRY_BYTES).contains(&max_entry_bytes)
            || max_entry_bytes > max_total_bytes
        {
            return Err(FetchProviderError::new(
                FetchProviderErrorCode::InvalidConfiguration,
            ));
        }
        Ok(Self {
            ttl,
            entries: max_entries,
            total_bytes: max_total_bytes,
            entry_bytes: max_entry_bytes,
        })
    }

    /// Returns the entry lifetime.
    #[must_use]
    pub const fn ttl(self) -> Duration {
        self.ttl
    }

    /// Returns the maximum entry count.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.entries
    }

    /// Returns the aggregate logical byte limit.
    #[must_use]
    pub const fn max_total_bytes(self) -> usize {
        self.total_bytes
    }

    /// Returns the per-entry logical byte limit.
    #[must_use]
    pub const fn max_entry_bytes(self) -> usize {
        self.entry_bytes
    }
}

impl Default for FetchCacheConfig {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_FETCH_CACHE_TTL,
            entries: DEFAULT_FETCH_CACHE_ENTRIES,
            total_bytes: DEFAULT_FETCH_CACHE_TOTAL_BYTES,
            entry_bytes: DEFAULT_FETCH_CACHE_ENTRY_BYTES,
        }
    }
}

/// Cache isolation boundary supplied by the upper-layer workspace and profile.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchCacheScope {
    workspace: String,
    profile: String,
}

impl FetchCacheScope {
    /// Creates an opaque bounded workspace/profile cache scope.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, NUL-containing, or control-containing values.
    pub fn new(
        workspace: impl Into<String>,
        profile: impl Into<String>,
    ) -> Result<Self, FetchProviderError> {
        let workspace = validate_scope_component(workspace.into())?;
        let profile = validate_scope_component(profile.into())?;
        Ok(Self { workspace, profile })
    }
}

impl fmt::Debug for FetchCacheScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchCacheScope")
            .field("workspace_bytes", &self.workspace.len())
            .field("profile_bytes", &self.profile.len())
            .finish_non_exhaustive()
    }
}

fn validate_scope_component(value: String) -> Result<String, FetchProviderError> {
    if value.is_empty()
        || value.len() > MAX_FETCH_CACHE_SCOPE_BYTES
        || value.chars().any(char::is_control)
    {
        Err(FetchProviderError::new(
            FetchProviderErrorCode::InvalidConfiguration,
        ))
    } else {
        Ok(value)
    }
}

/// Non-sensitive fetch cache occupancy snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchCacheStats {
    entries: usize,
    logical_bytes: usize,
}

impl FetchCacheStats {
    /// Returns the current entry count.
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the current bounded logical byte count.
    #[must_use]
    pub const fn logical_bytes(self) -> usize {
        self.logical_bytes
    }
}

/// Shared bounded in-memory cache of normalized fetch results.
pub struct FetchResultCache {
    config: FetchCacheConfig,
    state: Mutex<CacheState>,
}

impl FetchResultCache {
    /// Creates an empty normalized-result cache.
    #[must_use]
    pub fn new(config: FetchCacheConfig) -> Self {
        Self {
            config,
            state: Mutex::new(CacheState::default()),
        }
    }

    /// Returns a non-sensitive occupancy snapshot after purging expired entries.
    #[must_use]
    pub fn stats(&self) -> FetchCacheStats {
        let mut state = self.lock_state();
        state.purge_expired(Instant::now());
        FetchCacheStats {
            entries: state.entries.len(),
            logical_bytes: state.logical_bytes,
        }
    }

    /// Removes every cache entry without exposing its key or body.
    pub fn clear(&self) {
        *self.lock_state() = CacheState::default();
    }

    pub(crate) fn get(&self, key: &FetchCacheKey) -> Option<FetchResult> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &FetchCacheKey, now: Instant) -> Option<FetchResult> {
        let mut state = self.lock_state();
        state.purge_expired(now);
        if !state.entries.contains_key(key) {
            return None;
        }
        let access = state.next_access();
        let entry = state
            .entries
            .get_mut(key)
            .expect("cache key existence was checked while holding the lock");
        entry.last_access = access;
        Some(entry.result.clone())
    }

    pub(crate) fn insert(&self, key: FetchCacheKey, result: FetchResult) {
        self.insert_at(key, result, Instant::now());
    }

    fn insert_at(&self, key: FetchCacheKey, result: FetchResult, now: Instant) {
        let logical_bytes = key
            .logical_bytes()
            .saturating_add(result_logical_bytes(&result));
        if logical_bytes > self.config.max_entry_bytes()
            || logical_bytes > self.config.max_total_bytes()
        {
            return;
        }

        let mut state = self.lock_state();
        state.purge_expired(now);
        state.remove(&key);
        while state.entries.len() >= self.config.max_entries()
            || state.logical_bytes.saturating_add(logical_bytes) > self.config.max_total_bytes()
        {
            if !state.evict_least_recently_used() {
                break;
            }
        }
        let last_access = state.next_access();
        state.logical_bytes = state.logical_bytes.saturating_add(logical_bytes);
        state.entries.insert(
            key,
            CacheEntry {
                result,
                expires_at: now + self.config.ttl(),
                last_access,
                logical_bytes,
            },
        );
    }

    fn lock_state(&self) -> MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for FetchResultCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchResultCache")
            .field("config", &self.config)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FetchCacheKey {
    scope: FetchCacheScope,
    url: String,
    max_chars: usize,
    policy: FetchCachePolicyKey,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FetchCachePolicyKey {
    policy_version: u32,
    http_config: FetchHttpConfig,
    body_limits: FetchBodyLimits,
    accept: &'static str,
    accept_encoding: &'static str,
}

impl FetchCacheKey {
    pub(crate) fn new(
        scope: FetchCacheScope,
        url: String,
        max_chars: usize,
        policy: FetchCachePolicyKey,
    ) -> Self {
        Self {
            scope,
            url,
            max_chars,
            policy,
        }
    }

    fn logical_bytes(&self) -> usize {
        CACHE_ENTRY_OVERHEAD_BYTES
            .saturating_add(self.scope.workspace.len())
            .saturating_add(self.scope.profile.len())
            .saturating_add(self.url.len())
            .saturating_add(self.policy.accept.len())
            .saturating_add(self.policy.accept_encoding.len())
            .saturating_add(size_of::<usize>())
            .saturating_add(size_of::<FetchCachePolicyKey>())
    }
}

impl FetchCachePolicyKey {
    pub(crate) const fn new(
        policy_version: u32,
        http_config: FetchHttpConfig,
        body_limits: FetchBodyLimits,
        accept: &'static str,
        accept_encoding: &'static str,
    ) -> Self {
        Self {
            policy_version,
            http_config,
            body_limits,
            accept,
            accept_encoding,
        }
    }
}

#[derive(Default)]
struct CacheState {
    entries: BTreeMap<FetchCacheKey, CacheEntry>,
    logical_bytes: usize,
    access: u64,
}

impl CacheState {
    fn purge_expired(&mut self, now: Instant) {
        let expired = self
            .entries
            .iter()
            .filter_map(|(key, entry)| (entry.expires_at <= now).then_some(key.clone()))
            .collect::<Vec<_>>();
        for key in expired {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &FetchCacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.logical_bytes = self.logical_bytes.saturating_sub(entry.logical_bytes);
        }
    }

    fn evict_least_recently_used(&mut self) -> bool {
        let key = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(key, _)| key.clone());
        if let Some(key) = key {
            self.remove(&key);
            true
        } else {
            false
        }
    }

    fn next_access(&mut self) -> u64 {
        if self.access == u64::MAX {
            for (index, entry) in self.entries.values_mut().enumerate() {
                entry.last_access = u64::try_from(index).unwrap_or(u64::MAX - 1);
            }
            self.access = u64::try_from(self.entries.len()).unwrap_or(u64::MAX - 1);
        }
        let access = self.access;
        self.access = self.access.saturating_add(1);
        access
    }
}

struct CacheEntry {
    result: FetchResult,
    expires_at: Instant,
    last_access: u64,
    logical_bytes: usize,
}

fn result_logical_bytes(result: &FetchResult) -> usize {
    result
        .requested_url()
        .len()
        .saturating_add(result.final_url().len())
        .saturating_add(result.title().map_or(0, str::len))
        .saturating_add(result.mime_type().len())
        .saturating_add(result.body().len())
        .saturating_add(
            result
                .redirects()
                .iter()
                .map(|redirect| {
                    redirect
                        .from()
                        .len()
                        .saturating_add(redirect.to().len())
                        .saturating_add(size_of::<u16>())
                })
                .fold(0_usize, usize::saturating_add),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> FetchCacheScope {
        FetchCacheScope::new("workspace", "profile").unwrap()
    }

    fn key(policy_version: u32, accept_encoding: &'static str) -> FetchCacheKey {
        FetchCacheKey::new(
            scope(),
            "https://example.com/guide".to_owned(),
            100,
            FetchCachePolicyKey::new(
                policy_version,
                FetchHttpConfig::production(),
                FetchBodyLimits::default(),
                "text/plain",
                accept_encoding,
            ),
        )
    }

    fn result() -> FetchResult {
        FetchResult::new(
            "https://example.com/guide",
            "https://example.com/guide",
            "text/plain; charset=utf-8",
            "cached body",
        )
        .unwrap()
    }

    #[test]
    fn expiry_policy_version_and_negotiation_invalidate_entries_without_sleeping() {
        let ttl = Duration::from_secs(5);
        let cache = FetchResultCache::new(FetchCacheConfig::new(ttl, 4, 4096, 2048).unwrap());
        let now = Instant::now();
        let cached_key = key(1, "identity");
        cache.insert_at(cached_key.clone(), result(), now);

        assert!(cache.get_at(&cached_key, now).is_some());
        assert!(cache.get_at(&key(2, "identity"), now).is_none());
        assert!(cache.get_at(&key(1, "gzip"), now).is_none());
        assert!(cache.get_at(&cached_key, now + ttl).is_none());
        assert_eq!(cache.stats().entries(), 0);
    }
}
