use crate::{
    cache::{CacheEntry, TranslationCache},
    config::{Config, ResilienceConfig},
    detector::{detect_language, Language},
    error::{Result, TokenSaverError},
    preserver::{extract_and_preserve_with_config, restore_preserved},
    resilience::{CircuitBreaker, CircuitBreakerStats, RateLimiter},
    tokenizer::count_tokens,
};
use chrono::Utc;
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

const GOOGLE_TRANSLATE_URL: &str = "https://translate.googleapis.com/translate_a/single";

/// Maximum chunk size for translation (Google Translate limit is ~5000 chars)
const MAX_CHUNK_SIZE: usize = 4500;

/// Normalize whitespace by collapsing multiple whitespace to single spaces.
/// This is preserve-aware: should only be called on text with placeholders,
/// so code blocks and other preserved content are protected.
fn normalize_whitespace_internal(s: &str) -> String {
    let mut output = String::with_capacity(s.len());
    let mut iter = s.split_whitespace();
    if let Some(first) = iter.next() {
        output.push_str(first);
        for word in iter {
            output.push(' ');
            output.push_str(word);
        }
    }
    output
}

/// Maximum concurrent translation requests (rate limiting)
/// Keep conservative to avoid Google 429 rate limit errors
const MAX_CONCURRENT_TRANSLATIONS: usize = 5;

/// Global circuit breaker for Google Translate API
static CIRCUIT_BREAKER: OnceLock<CircuitBreaker> = OnceLock::new();

/// Global rate limiter for backpressure handling
static RATE_LIMITER: OnceLock<RateLimiter> = OnceLock::new();

/// Get or initialize the circuit breaker with default config
fn get_circuit_breaker() -> &'static CircuitBreaker {
    CIRCUIT_BREAKER.get_or_init(|| CircuitBreaker::new(&ResilienceConfig::default()))
}

/// Get or initialize the rate limiter
fn get_rate_limiter() -> &'static RateLimiter {
    RATE_LIMITER.get_or_init(RateLimiter::new)
}

/// Counter for User-Agent rotation
static UA_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Pool of User-Agent strings to rotate through
/// Helps avoid detection as automated traffic
const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:121.0) Gecko/20100101 Firefox/121.0",
];

/// Get next User-Agent string (round-robin rotation)
fn get_user_agent() -> &'static str {
    let idx = UA_COUNTER.fetch_add(1, Ordering::Relaxed) % USER_AGENTS.len();
    USER_AGENTS[idx]
}

/// Shared HTTP client with connection pooling, keep-alive, and HTTP/2
///
/// Benefits:
/// - Connection reuse: avoids repeated TLS handshakes and DNS lookups
/// - Keep-alive: maintains persistent connections to Google Translate
/// - Pool management: idle connections >= concurrent requests for optimal reuse
/// - HTTP/2: multiplexed requests over single connection (reduced latency)
/// - Gzip/Brotli: automatic response decompression (reduced bandwidth)
/// - TCP_NODELAY: reduced latency for small requests
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Get or initialize the shared HTTP client
fn get_http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5)) // Fail fast, let retry handle transient issues
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(MAX_CONCURRENT_TRANSLATIONS + 2) // >= concurrent for optimal reuse
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true) // Reduce latency for small requests
            .http2_adaptive_window(true) // Enable HTTP/2 with adaptive flow control
            .gzip(true) // Enable gzip decompression
            .brotli(true) // Enable brotli decompression
            .build()
            .expect("Failed to create HTTP client")
    })
}

/// Split text into chunks at natural boundaries
///
/// Uses single-pass reverse iteration for efficiency.
/// Priority: CJK sentence endings > Western sentences > newlines > spaces
fn chunk_text(text: &str) -> Vec<&str> {
    if text.len() <= MAX_CHUNK_SIZE {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= MAX_CHUNK_SIZE {
            chunks.push(remaining);
            break;
        }

        let split_pos = find_split_point_single_pass(remaining);
        chunks.push(&remaining[..split_pos]);
        remaining = &remaining[split_pos..];
    }

    chunks
}

/// Find optimal split point using single-pass reverse iteration
///
/// Scans backwards from MAX_CHUNK_SIZE, tracking the best split candidate
/// at each priority level. Avoids multiple string scans.
fn find_split_point_single_pass(text: &str) -> usize {
    // Find safe end at char boundary
    let mut safe_end = MAX_CHUNK_SIZE.min(text.len());
    while safe_end > 0 && !text.is_char_boundary(safe_end) {
        safe_end -= 1;
    }
    if safe_end == 0 {
        return text.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }

    // Track best split point at each priority level
    let mut best_cjk_sentence: Option<usize> = None; // Priority 1: 。！？
    let mut best_western_sentence: Option<usize> = None; // Priority 2: . ! ? (followed by space)
    let mut best_newline: Option<usize> = None; // Priority 3: \n
    let mut best_space: Option<usize> = None; // Priority 4: space

    let search_bytes = &text.as_bytes()[..safe_end];

    // Single reverse pass through characters
    for (char_idx, ch) in text[..safe_end].char_indices().rev() {
        let byte_pos = char_idx + ch.len_utf8();

        match ch {
            // CJK sentence endings (highest priority)
            '。' | '！' | '？' | '｡' => {
                if best_cjk_sentence.is_none() {
                    best_cjk_sentence = Some(byte_pos);
                }
            }
            // Western sentence endings (only if followed by whitespace or at end)
            '.' | '!' | '?' => {
                if best_western_sentence.is_none() {
                    // Check if at end or followed by whitespace (including \r for Windows newlines)
                    if byte_pos >= safe_end || {
                        let next_byte = search_bytes.get(byte_pos).copied().unwrap_or(b' ');
                        next_byte == b' '
                            || next_byte == b'\n'
                            || next_byte == b'\t'
                            || next_byte == b'\r'
                    } {
                        best_western_sentence = Some(byte_pos);
                    }
                }
            }
            // Newline
            '\n' => {
                if best_newline.is_none() {
                    best_newline = Some(byte_pos);
                }
            }
            // Space (word boundary)
            ' ' | '\t' => {
                if best_space.is_none() {
                    best_space = Some(byte_pos);
                }
            }
            _ => {}
        }

        // Early exit if we found highest priority split
        if best_cjk_sentence.is_some() {
            break;
        }
    }

    // Return best split point by priority
    best_cjk_sentence
        .or(best_western_sentence)
        .or(best_newline)
        .or(best_space)
        .unwrap_or(safe_end)
}

/// Translate multiple chunks concurrently with rate limiting and retry
///
/// Uses `buffered()` instead of `buffer_unordered()` to preserve chunk order.
/// This is critical for correctness - translations must be reassembled in order.
/// Each chunk has retry with exponential backoff for transient failures.
async fn translate_chunks(chunks: Vec<&str>, source_lang: Language) -> Result<Vec<String>> {
    use futures::stream::{self, StreamExt};

    let results: Vec<Result<String>> = stream::iter(chunks)
        .map(|chunk| async move { google_translate_with_retry(chunk, source_lang).await })
        .buffered(MAX_CONCURRENT_TRANSLATIONS) // buffered preserves order, buffer_unordered does not!
        .collect()
        .await;

    // Collect results, propagating first error
    results.into_iter().collect()
}

/// Translate with exponential backoff retry for transient failures
///
/// Features:
/// - Circuit breaker prevents cascading failures
/// - Rate limiter handles backpressure from 429 responses
/// - Exponential backoff with jitter to prevent thundering herd
/// - Configurable retry attempts and delays
async fn google_translate_with_retry(text: &str, source_lang: Language) -> Result<String> {
    let config = ResilienceConfig::default();
    google_translate_with_retry_config(text, source_lang, &config).await
}

/// Translate with retry using explicit config
async fn google_translate_with_retry_config(
    text: &str,
    source_lang: Language,
    config: &ResilienceConfig,
) -> Result<String> {
    let cb = get_circuit_breaker();
    let rl = get_rate_limiter();

    // Check circuit breaker first
    if !cb.allow_request() {
        return Err(TokenSaverError::CircuitOpen(
            config.circuit_breaker_reset_secs,
        ));
    }

    let mut last_error = None;

    for attempt in 0..config.max_retries {
        // Apply rate limiting backpressure
        rl.wait_if_needed().await;

        match google_translate(text, source_lang).await {
            Ok(result) => {
                // Success - record for circuit breaker and rate limiter
                cb.record_success();
                rl.record_success();
                return Ok(result);
            }
            Err(e) => {
                // Handle rate limiting specifically - extract Retry-After if available
                if let Some(retry_after) = e.retry_after_secs() {
                    rl.record_rate_limit(Some(retry_after));
                } else if matches!(e, TokenSaverError::RateLimited { .. }) {
                    rl.record_rate_limit(None);
                }

                // Check if error is retryable
                let is_retryable = e.is_retryable();

                if !is_retryable || attempt == config.max_retries - 1 {
                    // Record failure for circuit breaker
                    cb.record_failure();
                    return Err(e);
                }

                last_error = Some(e);

                // Exponential backoff with jitter: base * 2^attempt + random(0..100)
                // Jitter prevents thundering herd when multiple requests fail simultaneously
                let base_delay = config.retry_base_delay_ms * (1u64 << attempt);
                let jitter = fastrand::u64(0..100);
                tokio::time::sleep(Duration::from_millis(base_delay + jitter)).await;
            }
        }
    }

    // All retries exhausted
    cb.record_failure();
    Err(last_error.unwrap_or_else(|| TokenSaverError::Translation("Max retries exceeded".into())))
}

/// Translate text, automatically chunking if too long
async fn translate_with_chunking(text: &str, source_lang: Language) -> Result<String> {
    let chunks = chunk_text(text);

    if chunks.len() == 1 {
        // Single chunk, translate directly (with retry)
        return google_translate_with_retry(chunks[0], source_lang).await;
    }

    // Multiple chunks, translate in parallel and join
    let translated_chunks = translate_chunks(chunks, source_lang).await?;
    Ok(translated_chunks.join(""))
}

#[derive(Debug)]
pub struct TranslationResult {
    pub original: String,
    pub translated: String,
    pub was_translated: bool,
    pub source_language: Language,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_hit: bool,
}

/// Translate with explicit cache control
pub async fn translate_to_english_with_options(
    text: &str,
    config: &Config,
    use_cache: bool,
) -> Result<TranslationResult> {
    let detection = detect_language(text);

    // Check threshold - skip if below or already English
    if detection.ratio < config.threshold || detection.language == Language::English {
        return Ok(TranslationResult {
            original: text.to_string(),
            translated: text.to_string(),
            was_translated: false,
            source_language: detection.language,
            input_tokens: 0,
            output_tokens: 0,
            cache_hit: false,
        });
    }

    // Preserve code/URLs/markers before translation
    let preserve_config = (&config.preserve).into();
    let preserved = extract_and_preserve_with_config(text, &preserve_config);

    // Apply whitespace normalization to placeholder text (preserve-aware)
    // Uses Cow to avoid allocation when normalization is disabled
    let text_for_translation: Cow<str> = if config.normalize_whitespace {
        Cow::Owned(normalize_whitespace_internal(&preserved.text))
    } else {
        Cow::Borrowed(&preserved.text)
    };

    // Open cache once if enabled (reuse for both read and write)
    let cache = if use_cache && config.cache.enabled {
        TranslationCache::open(&config.cache).ok()
    } else {
        None
    };
    let cache_key =
        TranslationCache::make_key(detection.language.code(), "en", &text_for_translation);

    // Try cache lookup
    if let Some(ref c) = cache {
        if let Some(entry) = c.get(&cache_key) {
            // Cache hit - restore preserved segments and return
            let final_text = restore_preserved(&entry.translated, &preserved.segments);
            let input_tokens = count_tokens(text);
            let output_tokens = count_tokens(&final_text);

            return Ok(TranslationResult {
                original: text.to_string(),
                translated: final_text,
                was_translated: true,
                source_language: detection.language,
                input_tokens,
                output_tokens,
                cache_hit: true,
            });
        }
    }

    // Call Google Translate (with chunking for long inputs)
    let translated_text =
        translate_with_chunking(&text_for_translation, detection.language).await?;

    // Store in cache (reuse opened instance)
    if let Some(c) = cache {
        let entry = CacheEntry {
            translated: translated_text.clone(),
            timestamp: Utc::now().timestamp(),
            source_lang: detection.language.code().to_string(),
            target_lang: "en".to_string(),
        };
        c.put(&cache_key, &entry);
    }

    // Restore preserved segments
    let final_text = restore_preserved(&translated_text, &preserved.segments);

    // Count tokens using Claude's tokenizer
    let input_tokens = count_tokens(text);
    let output_tokens = count_tokens(&final_text);

    Ok(TranslationResult {
        original: text.to_string(),
        translated: final_text,
        was_translated: true,
        source_language: detection.language,
        input_tokens,
        output_tokens,
        cache_hit: false,
    })
}

async fn google_translate(text: &str, source_lang: Language) -> Result<String> {
    // Use shared HTTP client for connection pooling
    // Rotate User-Agent to avoid detection as automated traffic
    let response = get_http_client()
        .get(GOOGLE_TRANSLATE_URL)
        .query(&[
            ("client", "gtx"),
            ("sl", source_lang.code()),
            ("tl", "en"),
            ("dt", "t"),
            ("q", text),
        ])
        .header("User-Agent", get_user_agent())
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        // Extract Retry-After header for 429 responses
        let retry_after_secs = if status.as_u16() == 429 {
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
        } else {
            None
        };
        return Err(TokenSaverError::from_status_with_retry_after(
            status,
            retry_after_secs,
        ));
    }

    // Response is nested JSON array: [[["translated text","original",null,null,10],...],...]
    let body: serde_json::Value = response.json().await?;

    // Pre-allocate result string to avoid repeated reallocations
    // English translation is typically similar length to CJK input (+ margin)
    let mut result = String::with_capacity(text.len() + 32);
    if let Some(outer) = body.as_array() {
        if let Some(inner) = outer.first().and_then(|v| v.as_array()) {
            for item in inner {
                if let Some(translated) = item
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                {
                    result.push_str(translated);
                }
            }
        }
    }

    if result.is_empty() {
        return Err(TokenSaverError::Translation("Empty response".into()));
    }

    Ok(result)
}

/// Build instruction for Claude to respond in a specific language
pub fn build_output_language_instruction(output_lang: &str) -> String {
    match output_lang {
        "zh" | "zh-CN" | "zh-TW" => {
            "\n\n[IMPORTANT: Please respond in Chinese (请用中文回答)]".into()
        }
        "ja" => "\n\n[IMPORTANT: Please respond in Japanese (日本語で回答してください)]".into(),
        "ko" => "\n\n[IMPORTANT: Please respond in Korean (한국어로 답변해주세요)]".into(),
        _ => String::new(),
    }
}

/// Resilience statistics for monitoring
#[derive(Debug)]
pub struct ResilienceStats {
    pub circuit_breaker: CircuitBreakerStats,
    pub rate_limit_delay_ms: u64,
    pub rate_limit_hits: u32,
}

/// Get current resilience statistics for monitoring
pub fn get_resilience_stats() -> ResilienceStats {
    ResilienceStats {
        circuit_breaker: get_circuit_breaker().stats(),
        rate_limit_delay_ms: get_rate_limiter().current_delay_ms(),
        rate_limit_hits: get_rate_limiter().rate_limit_hits(),
    }
}

/// Reset resilience state (useful for testing or after configuration changes)
pub fn reset_resilience_state() {
    get_circuit_breaker().reset();
    get_rate_limiter().reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_language_instruction() {
        assert!(build_output_language_instruction("zh").contains("Chinese"));
        assert!(build_output_language_instruction("ja").contains("Japanese"));
        assert!(build_output_language_instruction("ko").contains("Korean"));
        assert!(build_output_language_instruction("en").is_empty());
    }

    #[test]
    fn test_chunk_text_short() {
        let text = "Hello world";
        let chunks = chunk_text(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_text_exactly_max_size() {
        let text = "a".repeat(MAX_CHUNK_SIZE);
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_text_splits_at_cjk_sentence() {
        // Create text that exceeds MAX_CHUNK_SIZE with CJK sentences
        let sentence = "これは日本語の文章です。";
        let repeat_count = MAX_CHUNK_SIZE / sentence.len() + 2;
        let text = sentence.repeat(repeat_count);

        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1, "Should split into multiple chunks");

        // Verify all chunks end at sentence boundaries (except possibly last)
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(
                chunk.ends_with('。') || chunk.ends_with('！') || chunk.ends_with('？'),
                "Chunk should end at sentence boundary: {:?}",
                chunk.chars().rev().take(5).collect::<String>()
            );
        }
    }

    #[test]
    fn test_chunk_text_preserves_all_content() {
        let text = "Hello. World! Test? ".repeat(500); // Exceeds MAX_CHUNK_SIZE
        let chunks = chunk_text(&text);
        let rejoined: String = chunks.into_iter().collect();
        assert_eq!(rejoined, text, "Chunks should rejoin to original");
    }

    #[test]
    fn test_chunk_text_handles_unicode() {
        // Mix of Korean, Japanese, Chinese - ensure no mid-char splits
        let text = "한글 테스트。日本語テスト。中文测试。".repeat(200);
        let chunks = chunk_text(&text);

        for chunk in &chunks {
            // All chunks should be valid UTF-8 (no panics)
            assert!(!chunk.is_empty());
            // Verify we can iterate chars without panic
            let _ = chunk.chars().count();
        }
    }

    #[test]
    fn test_chunk_text_no_empty_chunks() {
        let text = "Test sentence. ".repeat(500);
        let chunks = chunk_text(&text);

        for (i, chunk) in chunks.iter().enumerate() {
            assert!(!chunk.is_empty(), "Chunk {} should not be empty", i);
        }
    }

    #[test]
    fn test_chunk_text_windows_newlines() {
        // Windows-style CRLF newlines should be handled correctly
        let sentence = "Test sentence.\r\n";
        let repeat_count = MAX_CHUNK_SIZE / sentence.len() + 2;
        let text = sentence.repeat(repeat_count);

        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1, "Should split into multiple chunks");

        // Verify chunks split at sentence boundaries (after period, before \r\n)
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(
                chunk.ends_with(".\r\n") || chunk.ends_with('.'),
                "Chunk should end at sentence boundary: {:?}",
                chunk.chars().rev().take(10).collect::<String>()
            );
        }

        // Verify content is preserved
        let rejoined: String = chunks.into_iter().collect();
        assert_eq!(rejoined, text, "Chunks should rejoin to original");
    }

    #[test]
    fn test_normalize_whitespace_internal() {
        // Basic whitespace collapse
        assert_eq!(
            normalize_whitespace_internal("hello    world"),
            "hello world"
        );

        // Multiple types of whitespace
        assert_eq!(normalize_whitespace_internal("a  b\t\tc\n\nd"), "a b c d");

        // Already normalized
        assert_eq!(normalize_whitespace_internal("hello world"), "hello world");

        // Empty string
        assert_eq!(normalize_whitespace_internal(""), "");

        // Only whitespace
        assert_eq!(normalize_whitespace_internal("   \t\n  "), "");

        // Preserves placeholders (simulating preserved segments)
        let with_placeholder = "text \u{FEFF}cjkcode0\u{FEFF}  more    text";
        let normalized = normalize_whitespace_internal(with_placeholder);
        assert!(normalized.contains("\u{FEFF}cjkcode0\u{FEFF}"));
        assert_eq!(normalized, "text \u{FEFF}cjkcode0\u{FEFF} more text");
    }
}
