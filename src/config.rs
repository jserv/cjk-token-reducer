use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const CONFIG_FILENAME: &str = ".cjk-token.json";

/// Cache configuration with serde defaults
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,

    #[serde(default = "default_ttl_days")]
    pub ttl_days: u32,

    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
}

/// Resilience configuration for retry, timeout, and circuit breaker
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResilienceConfig {
    /// Request timeout in seconds (default: 30)
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    /// Connection timeout in seconds (default: 5)
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,

    /// Maximum retry attempts for transient failures (default: 3)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Base delay for exponential backoff in milliseconds (default: 200)
    #[serde(default = "default_retry_base_delay_ms")]
    pub retry_base_delay_ms: u64,

    /// Circuit breaker failure threshold before opening (default: 5)
    #[serde(default = "default_circuit_breaker_threshold")]
    pub circuit_breaker_threshold: u32,

    /// Circuit breaker reset timeout in seconds (default: 60)
    #[serde(default = "default_circuit_breaker_reset_secs")]
    pub circuit_breaker_reset_secs: u64,

    /// Enable graceful fallback to passthrough on failure (default: true)
    #[serde(default = "default_true")]
    pub fallback_to_passthrough: bool,
}

// Resilience defaults
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 5;
const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 200;
const DEFAULT_CIRCUIT_BREAKER_THRESHOLD: u32 = 5;
const DEFAULT_CIRCUIT_BREAKER_RESET_SECS: u64 = 60;

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}
fn default_connect_timeout_secs() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_SECS
}
fn default_max_retries() -> u32 {
    DEFAULT_MAX_RETRIES
}
fn default_retry_base_delay_ms() -> u64 {
    DEFAULT_RETRY_BASE_DELAY_MS
}
fn default_circuit_breaker_threshold() -> u32 {
    DEFAULT_CIRCUIT_BREAKER_THRESHOLD
}
fn default_circuit_breaker_reset_secs() -> u64 {
    DEFAULT_CIRCUIT_BREAKER_RESET_SECS
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_base_delay_ms: DEFAULT_RETRY_BASE_DELAY_MS,
            circuit_breaker_threshold: DEFAULT_CIRCUIT_BREAKER_THRESHOLD,
            circuit_breaker_reset_secs: DEFAULT_CIRCUIT_BREAKER_RESET_SECS,
            fallback_to_passthrough: true,
        }
    }
}

/// Preservation configuration for no-translate markers and term detection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreserveConfig {
    /// Enable [[...]] wiki-style no-translate markers (default: true)
    #[serde(default = "default_true")]
    pub wiki_markers: bool,

    /// Enable ==...== highlight-style no-translate markers (default: true)
    #[serde(default = "default_true")]
    pub highlight_markers: bool,

    /// Auto-preserve English technical terms like camelCase, SCREAMING_CASE (default: true)
    #[serde(default = "default_true")]
    pub english_terms: bool,
}

fn default_true() -> bool {
    true
}

impl Default for PreserveConfig {
    fn default() -> Self {
        Self {
            wiki_markers: true,
            highlight_markers: true,
            english_terms: true,
        }
    }
}

impl From<&PreserveConfig> for crate::preserver::PreserveConfig {
    fn from(config: &PreserveConfig) -> Self {
        crate::preserver::PreserveConfig {
            wiki_markers: config.wiki_markers,
            highlight_markers: config.highlight_markers,
            english_terms: config.english_terms,
        }
    }
}

// Cache defaults
const DEFAULT_CACHE_ENABLED: bool = true;
const DEFAULT_TTL_DAYS: u32 = 30;
const DEFAULT_MAX_SIZE_MB: u32 = 10;

fn default_cache_enabled() -> bool {
    DEFAULT_CACHE_ENABLED
}
fn default_ttl_days() -> u32 {
    DEFAULT_TTL_DAYS
}
fn default_max_size_mb() -> u32 {
    DEFAULT_MAX_SIZE_MB
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_CACHE_ENABLED,
            ttl_days: DEFAULT_TTL_DAYS,
            max_size_mb: DEFAULT_MAX_SIZE_MB,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_output_language")]
    pub output_language: String,

    #[serde(default = "default_enable_stats")]
    pub enable_stats: bool,

    #[serde(default = "default_threshold")]
    pub threshold: f64,

    /// Collapse internal whitespace to single spaces for token reduction.
    /// WARNING: This destroys code indentation. Only enable for non-code prompts.
    /// Default: false (safe)
    #[serde(default)]
    pub normalize_whitespace: bool,

    #[serde(default)]
    pub cache: CacheConfig,

    #[serde(default)]
    pub preserve: PreserveConfig,

    #[serde(default)]
    pub resilience: ResilienceConfig,
}

// Config defaults
const DEFAULT_OUTPUT_LANGUAGE: &str = "en";
const DEFAULT_ENABLE_STATS: bool = true;
const DEFAULT_THRESHOLD: f64 = 0.1;

fn default_output_language() -> String {
    DEFAULT_OUTPUT_LANGUAGE.into()
}
fn default_enable_stats() -> bool {
    DEFAULT_ENABLE_STATS
}
fn default_threshold() -> f64 {
    DEFAULT_THRESHOLD
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_language: DEFAULT_OUTPUT_LANGUAGE.into(),
            enable_stats: DEFAULT_ENABLE_STATS,
            threshold: DEFAULT_THRESHOLD,
            normalize_whitespace: false,
            cache: CacheConfig::default(),
            preserve: PreserveConfig::default(),
            resilience: ResilienceConfig::default(),
        }
    }
}

/// Load configuration from file, applying environment variable overrides
pub fn load_config() -> Config {
    let mut config: Config = find_config_file()
        .and_then(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            match serde_json::from_str(&content) {
                Ok(config) => Some(config),
                Err(e) => {
                    crate::output::print_error(&format!("Config parse error: {e}"));
                    None
                }
            }
        })
        .unwrap_or_default();

    // Apply environment variable overrides
    if let Ok(val) = std::env::var("CJK_TOKEN_OUTPUT_LANG") {
        config.output_language = val;
    }
    if let Ok(val) = std::env::var("CJK_TOKEN_THRESHOLD") {
        if let Ok(threshold) = val.parse::<f64>() {
            config.threshold = threshold;
        }
    }
    if let Ok(val) = std::env::var("CJK_TOKEN_CACHE_ENABLED") {
        config.cache.enabled = val.to_lowercase() == "true" || val == "1";
    }

    config
}

/// Search for config file in standard locations
fn find_config_file() -> Option<PathBuf> {
    let search_paths = [
        std::env::current_dir().ok(),
        dirs::home_dir(),
        dirs::config_dir().map(|p| p.join("cjk-token-reducer")),
    ];

    for base in search_paths.into_iter().flatten() {
        let config_path = base.join(CONFIG_FILENAME);
        if config_path.exists() {
            return Some(config_path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.output_language, "en");
        assert_eq!(config.threshold, 0.1);
        assert!(config.enable_stats);
        assert!(!config.normalize_whitespace); // default false for safety
    }

    #[test]
    fn test_normalize_whitespace_config() {
        // Default should be false (safe for code)
        let json = r#"{}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(!config.normalize_whitespace);

        // Can be enabled explicitly
        let json = r#"{"normalizeWhitespace": true}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.normalize_whitespace);
    }

    #[test]
    fn test_deserialize_partial() {
        let json = r#"{"threshold": 0.2}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.threshold, 0.2);
        assert_eq!(config.output_language, "en"); // default
    }

    #[test]
    fn test_preserve_config_defaults() {
        let config = PreserveConfig::default();
        assert!(config.wiki_markers);
        assert!(config.highlight_markers);
        assert!(config.english_terms);
    }

    #[test]
    fn test_preserve_config_deserialize_defaults() {
        // Empty JSON should use defaults (all true)
        let json = r#"{}"#;
        let config: PreserveConfig = serde_json::from_str(json).unwrap();
        assert!(config.wiki_markers);
        assert!(config.highlight_markers);
        assert!(config.english_terms);
    }

    #[test]
    fn test_preserve_config_partial_override() {
        // Partial config should override only specified fields
        let json = r#"{"wikiMarkers": false}"#;
        let config: PreserveConfig = serde_json::from_str(json).unwrap();
        assert!(!config.wiki_markers); // overridden
        assert!(config.highlight_markers); // default
        assert!(config.english_terms); // default
    }

    #[test]
    fn test_resilience_config_defaults() {
        let config = ResilienceConfig::default();
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.connect_timeout_secs, 5);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_base_delay_ms, 200);
        assert_eq!(config.circuit_breaker_threshold, 5);
        assert_eq!(config.circuit_breaker_reset_secs, 60);
        assert!(config.fallback_to_passthrough);
    }

    #[test]
    fn test_resilience_config_partial_override() {
        let json = r#"{"maxRetries": 5, "timeoutSecs": 60}"#;
        let config: ResilienceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.max_retries, 5); // overridden
        assert_eq!(config.timeout_secs, 60); // overridden
        assert_eq!(config.connect_timeout_secs, 5); // default
        assert_eq!(config.retry_base_delay_ms, 200); // default
    }

    #[test]
    fn test_config_includes_resilience() {
        let config = Config::default();
        assert_eq!(config.resilience.max_retries, 3);
        assert!(config.resilience.fallback_to_passthrough);
    }
}
