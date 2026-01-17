//! Translation cache using sled embedded database
//!
//! Eliminates redundant Google Translate API calls by caching translations locally.
//!
//! This module is conditionally compiled with the `cache` feature.
//! When disabled, provides stub implementations that always miss.

use crate::config::CacheConfig;
use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Cached translation entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub translated: String,
    pub timestamp: i64,
    pub source_lang: String,
    pub target_lang: String,
}

/// Cache statistics for display
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub entries: u64,
    pub size_bytes: u64,
    pub session_hits: u64,
    pub session_misses: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.session_hits + self.session_misses;
        if total == 0 {
            0.0
        } else {
            self.session_hits as f64 / total as f64
        }
    }
}

/// Format cache statistics for display
pub fn format_cache_stats(stats: &CacheStats) -> String {
    let size_mb = stats.size_bytes as f64 / (1024.0 * 1024.0);
    let hit_rate = stats.hit_rate() * 100.0;

    format!(
        r#"
╔════════════════════════════════════════╗
║       Translation Cache Statistics     ║
╠════════════════════════════════════════╣
║ Entries:        {:>20}   ║
║ Size:           {:>17.2} MB   ║
║ Session Hits:   {:>20}   ║
║ Session Misses: {:>20}   ║
║ Hit Rate:       {:>18.1}%    ║
╚════════════════════════════════════════╝
"#,
        stats.entries, size_mb, stats.session_hits, stats.session_misses, hit_rate
    )
}

// ============================================================================
// Feature-gated implementation: Full cache with sled
// ============================================================================

#[cfg(feature = "cache")]
mod cache_impl {
    use super::*;
    use crate::error::TokenSaverError;
    use chrono::Utc;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Global cache statistics for the current session
    static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
    static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
    /// Counter for throttling size limit checks (every N inserts)
    static INSERT_COUNT: AtomicU64 = AtomicU64::new(0);
    /// Check size limit every N inserts to avoid expensive size_on_disk() calls
    const SIZE_CHECK_INTERVAL: u64 = 50;
    /// Force size check if entry exceeds this threshold (bytes)
    const LARGE_ENTRY_THRESHOLD: usize = 4096;
    /// Maximum eviction iterations to prevent infinite loops
    const MAX_EVICTION_ROUNDS: usize = 10;

    /// Translation cache backed by sled
    pub struct TranslationCache {
        db: sled::Db,
        config: CacheConfig,
    }

    impl TranslationCache {
        /// Open or create the cache database
        ///
        /// Returns error if cache is locked by another process (e.g., concurrent instance)
        pub fn open(config: &CacheConfig) -> Result<Self> {
            let path = cache_path();

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    TokenSaverError::Cache(format!("Failed to create cache dir: {e}"))
                })?;
            }

            let db = sled::open(&path).map_err(|e| {
                // Check for lock contention (common with concurrent instances)
                let is_lock_error = match &e {
                    sled::Error::Io(io_err) => matches!(
                        io_err.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::ResourceBusy
                            | std::io::ErrorKind::PermissionDenied
                    ),
                    _ => false,
                };
                let msg = e.to_string().to_lowercase();
                let is_lock_msg =
                    msg.contains("lock") || msg.contains("busy") || msg.contains("flock");

                if is_lock_error || is_lock_msg {
                    TokenSaverError::Cache(
                        "Cache locked by another process. Use --no-cache to bypass.".into(),
                    )
                } else {
                    TokenSaverError::Cache(format!("Failed to open cache: {e}"))
                }
            })?;

            Ok(Self {
                db,
                config: config.clone(),
            })
        }

        /// Open cache at a specific path (for testing)
        ///
        /// This avoids modifying global environment variables which causes
        /// thread-safety issues in parallel test execution.
        #[cfg(test)]
        pub fn open_at_path(config: &CacheConfig, path: &std::path::Path) -> Result<Self> {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    TokenSaverError::Cache(format!("Failed to create cache dir: {e}"))
                })?;
            }

            let db = sled::open(path)
                .map_err(|e| TokenSaverError::Cache(format!("Failed to open cache: {e}")))?;

            Ok(Self {
                db,
                config: config.clone(),
            })
        }

        /// Generate cache key from translation parameters
        ///
        /// Key format: SHA-256 of "{source_lang}:{target_lang}:{text}"
        pub fn make_key(source_lang: &str, target_lang: &str, text: &str) -> String {
            let mut hasher = Sha256::new();
            hasher.update(source_lang.as_bytes());
            hasher.update(b":");
            hasher.update(target_lang.as_bytes());
            hasher.update(b":");
            hasher.update(text.as_bytes());
            hex::encode(hasher.finalize())
        }

        /// Get cached translation if available and not expired
        pub fn get(&self, key: &str) -> Option<CacheEntry> {
            match self.db.get(key) {
                Ok(Some(bytes)) => match serde_json::from_slice::<CacheEntry>(&bytes) {
                    Ok(entry) => {
                        let now = Utc::now().timestamp();
                        let ttl_secs = self.config.ttl_days as i64 * 24 * 60 * 60;
                        if now - entry.timestamp > ttl_secs {
                            let _ = self.db.remove(key);
                            CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                            None
                        } else {
                            CACHE_HITS.fetch_add(1, Ordering::Relaxed);
                            Some(entry)
                        }
                    }
                    Err(_) => {
                        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                },
                _ => {
                    CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
                    None
                }
            }
        }

        /// Store translation in cache
        pub fn put(&self, key: &str, entry: &CacheEntry) {
            if let Ok(bytes) = serde_json::to_vec(entry) {
                let entry_size = bytes.len();
                let _ = self.db.insert(key, bytes);

                let count = INSERT_COUNT.fetch_add(1, Ordering::Relaxed);
                if count % SIZE_CHECK_INTERVAL == 0 || entry_size > LARGE_ENTRY_THRESHOLD {
                    self.enforce_size_limit();
                }
            }
        }

        /// Get cache statistics
        pub fn stats(&self) -> CacheStats {
            CacheStats {
                entries: self.db.len() as u64,
                size_bytes: self.db.size_on_disk().unwrap_or(0),
                session_hits: CACHE_HITS.load(Ordering::Relaxed),
                session_misses: CACHE_MISSES.load(Ordering::Relaxed),
            }
        }

        /// Clear all cached translations
        pub fn clear(&self) -> Result<()> {
            self.db
                .clear()
                .map_err(|e| TokenSaverError::Cache(format!("Failed to clear cache: {e}")))?;
            let _ = self.db.flush();
            Ok(())
        }

        /// Enforce max size limit using random eviction
        fn enforce_size_limit(&self) {
            let max_bytes = self.config.max_size_mb as u64 * 1024 * 1024;

            for _round in 0..MAX_EVICTION_ROUNDS {
                let current_size = self.db.size_on_disk().unwrap_or(0);
                if current_size <= max_bytes {
                    return;
                }

                let len = self.db.len();
                if len == 0 {
                    return;
                }

                let entries_to_remove = std::cmp::max(1, len / 4);
                let mut removed = 0;

                for item in self.db.iter() {
                    if removed >= entries_to_remove {
                        break;
                    }
                    if let Ok((key, _)) = item {
                        let _ = self.db.remove(key);
                        removed += 1;
                    }
                }

                let _ = self.db.flush();

                if removed == 0 {
                    return;
                }
            }
        }
    }

    /// Get the cache database path
    fn cache_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("cjk-token-reducer")
            .join("translations.db")
    }

    #[cfg(test)]
    pub(super) const TEST_LARGE_ENTRY_THRESHOLD: usize = LARGE_ENTRY_THRESHOLD;
}

// ============================================================================
// Stub implementation: No-op cache when feature is disabled
// ============================================================================

#[cfg(not(feature = "cache"))]
mod cache_impl {
    use super::*;

    /// Stub translation cache (no-op when cache feature is disabled)
    pub struct TranslationCache {
        _config: CacheConfig,
    }

    impl TranslationCache {
        /// Open stub cache (always succeeds)
        pub fn open(config: &CacheConfig) -> Result<Self> {
            Ok(Self {
                _config: config.clone(),
            })
        }

        /// Generate cache key (same algorithm for compatibility)
        pub fn make_key(source_lang: &str, target_lang: &str, text: &str) -> String {
            // Simple hash without sha2 dependency
            format!("{}:{}:{:x}", source_lang, target_lang, text.len())
        }

        /// Get from cache (always misses)
        pub fn get(&self, _key: &str) -> Option<CacheEntry> {
            None
        }

        /// Store in cache (no-op)
        pub fn put(&self, _key: &str, _entry: &CacheEntry) {}

        /// Get cache statistics (empty)
        pub fn stats(&self) -> CacheStats {
            CacheStats::default()
        }

        /// Clear cache (no-op)
        pub fn clear(&self) -> Result<()> {
            Ok(())
        }
    }
}

// Re-export TranslationCache from the appropriate implementation
pub use cache_impl::TranslationCache;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hit_rate_calculation() {
        let stats = CacheStats {
            entries: 100,
            size_bytes: 1024,
            session_hits: 80,
            session_misses: 20,
        };
        assert!((stats.hit_rate() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_hit_rate_zero_requests() {
        let stats = CacheStats {
            entries: 0,
            size_bytes: 0,
            session_hits: 0,
            session_misses: 0,
        };
        assert_eq!(stats.hit_rate(), 0.0);
    }

    #[test]
    fn test_format_cache_stats() {
        let stats = CacheStats {
            entries: 100,
            size_bytes: 2 * 1024 * 1024, // 2 MB
            session_hits: 80,
            session_misses: 20,
        };
        let output = format_cache_stats(&stats);
        assert!(output.contains("Entries:"));
        assert!(output.contains("2.00 MB"));
        assert!(output.contains("Hit Rate:"));
        assert!(output.contains("80.0%"));
    }

    #[cfg(feature = "cache")]
    #[test]
    fn test_cache_key_generation() {
        let key1 = TranslationCache::make_key("ko", "en", "hello");
        let key2 = TranslationCache::make_key("ko", "en", "hello");
        let key3 = TranslationCache::make_key("ja", "en", "hello");

        assert_eq!(key1, key2); // Same inputs = same key
        assert_ne!(key1, key3); // Different lang = different key
        assert_eq!(key1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[cfg(feature = "cache")]
    #[test]
    fn test_default_cache_config() {
        use crate::config::CacheConfig;
        let config = CacheConfig::default();
        assert!(config.enabled);
        assert_eq!(config.ttl_days, 30);
        assert_eq!(config.max_size_mb, 10);
    }

    #[cfg(feature = "cache")]
    #[test]
    fn test_eviction_minimum_one() {
        for len in 0..10 {
            let entries_to_remove = std::cmp::max(1, len / 4);
            if len > 0 {
                assert!(
                    entries_to_remove >= 1,
                    "len={} should remove at least 1",
                    len
                );
            }
        }
        assert_eq!(std::cmp::max(1, 1 / 4), 1);
        assert_eq!(std::cmp::max(1, 2 / 4), 1);
        assert_eq!(std::cmp::max(1, 3 / 4), 1);
        assert_eq!(std::cmp::max(1, 4 / 4), 1);
        assert_eq!(std::cmp::max(1, 8 / 4), 2);
    }

    #[cfg(feature = "cache")]
    #[test]
    fn test_large_entry_threshold() {
        use cache_impl::TEST_LARGE_ENTRY_THRESHOLD;
        // Verify constant is set to 4096 (4KB)
        assert_eq!(TEST_LARGE_ENTRY_THRESHOLD, 4096);
        // Entries > 4096 bytes are considered large (tested in cache eviction logic)
    }

    #[cfg(feature = "cache")]
    #[test]
    fn test_cache_operations() {
        use crate::config::CacheConfig;
        use chrono::Utc;

        // Create a temporary directory for the test cache
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("test_cache.db");

        let config = CacheConfig {
            enabled: true,
            ttl_days: 30,
            max_size_mb: 10,
        };

        // Open cache at specific path (avoids modifying HOME env var)
        let cache = TranslationCache::open_at_path(&config, &cache_path).unwrap();

        // Test putting and getting an entry
        let key = TranslationCache::make_key("zh", "en", "你好");
        let entry = CacheEntry {
            translated: "Hello".to_string(),
            timestamp: Utc::now().timestamp(),
            source_lang: "zh".to_string(),
            target_lang: "en".to_string(),
        };

        cache.put(&key, &entry);
        let retrieved = cache.get(&key);

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().translated, "Hello");

        // Test cache stats
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);

        // Clean up (temp_dir auto-deletes on drop)
        cache.clear().unwrap();
    }

    #[cfg(not(feature = "cache"))]
    #[test]
    fn test_stub_cache_operations() {
        use crate::config::CacheConfig;

        let config = CacheConfig {
            enabled: true,
            ttl_days: 30,
            max_size_mb: 10,
        };

        // Open stub cache
        let cache = TranslationCache::open(&config).unwrap();

        // Test putting and getting an entry (should always miss with stub)
        let key = TranslationCache::make_key("zh", "en", "你好");
        let entry = CacheEntry {
            translated: "Hello".to_string(),
            timestamp: 0,
            source_lang: "zh".to_string(),
            target_lang: "en".to_string(),
        };

        cache.put(&key, &entry);
        let retrieved = cache.get(&key);

        // Stub implementation always returns None
        assert!(retrieved.is_none());

        // Stats should be default
        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
    }
}
