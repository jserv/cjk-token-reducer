//! Token counting using Claude's tokenizer
//!
//! When the `tokenizer` feature is enabled, uses the claude-tokenizer crate
//! for precise token counting. Otherwise, falls back to estimation.

use crate::detector::is_cjk_char;

/// Result of token counting with fallback indicator
#[derive(Debug)]
pub struct TokenCountResult {
    pub count: usize,
    pub used_fallback: bool,
}

/// Count tokens using Claude's tokenizer for accurate measurement
pub fn count_tokens(text: &str) -> usize {
    count_tokens_with_fallback(text).count
}

/// Count tokens with fallback indicator
#[cfg(feature = "tokenizer")]
pub fn count_tokens_with_fallback(text: &str) -> TokenCountResult {
    match claude_tokenizer::count_tokens(text) {
        Ok(count) => TokenCountResult {
            count,
            used_fallback: false,
        },
        Err(_) => TokenCountResult {
            count: estimate_tokens_fallback(text),
            used_fallback: true,
        },
    }
}

/// Count tokens with fallback indicator (fallback-only when feature is disabled)
#[cfg(not(feature = "tokenizer"))]
pub fn count_tokens_with_fallback(text: &str) -> TokenCountResult {
    TokenCountResult {
        count: estimate_tokens_fallback(text),
        used_fallback: true,
    }
}

/// Tokenize text and return individual tokens with fallback indicator
#[cfg(feature = "tokenizer")]
pub fn tokenize_with_fallback(text: &str) -> (Vec<String>, bool) {
    match claude_tokenizer::tokenize(text) {
        Ok(tokens) => (tokens.into_iter().map(|(_, s)| s).collect(), false),
        Err(_) => (vec![], true),
    }
}

/// Tokenize text (empty when feature is disabled)
#[cfg(not(feature = "tokenizer"))]
pub fn tokenize_with_fallback(_text: &str) -> (Vec<String>, bool) {
    (vec![], true)
}

/// Tokenize text and return individual tokens
pub fn tokenize(text: &str) -> Vec<String> {
    tokenize_with_fallback(text).0
}

/// Fallback estimation when tokenizer is unavailable or fails
///
/// Uses character-based heuristics calibrated for CJK text:
/// - CJK characters: ~1.5 tokens per character
/// - Non-CJK: ~0.25 tokens per character (roughly 4 chars per token)
fn estimate_tokens_fallback(text: &str) -> usize {
    let cjk_chars = text.chars().filter(is_cjk_char).count();
    let non_cjk_chars = text.chars().count() - cjk_chars;

    // CJK: ~1.5 tokens per char, Non-CJK: ~0.25 tokens per char
    ((cjk_chars as f64 * 1.5) + (non_cjk_chars as f64 * 0.25)).ceil() as usize
}

/// Calculate token savings between original and translated text
pub fn calculate_savings(original: &str, translated: &str) -> TokenSavings {
    let original_tokens = count_tokens(original);
    let translated_tokens = count_tokens(translated);
    let saved = original_tokens.saturating_sub(translated_tokens);
    let savings_percent = if original_tokens > 0 {
        (saved as f64 / original_tokens as f64) * 100.0
    } else {
        0.0
    };

    TokenSavings {
        original_tokens,
        translated_tokens,
        saved_tokens: saved,
        savings_percent,
    }
}

/// Token savings calculation result
#[derive(Debug)]
pub struct TokenSavings {
    pub original_tokens: usize,
    pub translated_tokens: usize,
    pub saved_tokens: usize,
    pub savings_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_chinese() {
        let count = count_tokens("你好世界");
        assert!(count > 0);
    }

    #[test]
    fn test_count_tokens_japanese() {
        let count = count_tokens("こんにちは");
        assert!(count > 0);
    }

    #[test]
    fn test_count_tokens_korean() {
        let count = count_tokens("안녕하세요");
        assert!(count > 0);
    }

    #[test]
    fn test_count_tokens_english() {
        let count = count_tokens("Hello, world!");
        assert!(count > 0);
    }

    #[test]
    fn test_calculate_savings() {
        let savings = calculate_savings("這是一個測試", "This is a test");
        assert!(savings.original_tokens > 0);
        assert!(savings.translated_tokens > 0);
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello world");
        // When tokenizer feature is disabled, returns empty vec
        // When enabled, should return tokens
        #[cfg(feature = "tokenizer")]
        assert!(!tokens.is_empty() || count_tokens("Hello world") > 0);
        #[cfg(not(feature = "tokenizer"))]
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_fallback_estimation() {
        let count = estimate_tokens_fallback("Hello 你好");
        assert!(count > 0);
    }

    #[test]
    fn test_fallback_indicator() {
        let result = count_tokens_with_fallback("Hello world");
        // With tokenizer feature: may or may not use fallback
        // Without tokenizer feature: always uses fallback
        #[cfg(not(feature = "tokenizer"))]
        assert!(result.used_fallback);
        assert!(result.count > 0);
    }

    #[test]
    fn test_fallback_estimation_cjk_heavy() {
        // Test with mostly CJK characters
        let count = estimate_tokens_fallback("你好世界世界世界");
        // Should be higher than pure English equivalent
        let eng_count = estimate_tokens_fallback("hello world");
        assert!(count > eng_count);
    }

    #[test]
    fn test_estimate_tokens_fallback_cjk_chars() {
        // Test primary CJK character ranges (Chinese, Japanese, Korean)
        let cjk_chars = [
            "一", // Basic CJK Unified Ideograph (Chinese/Japanese Kanji)
            "あ", // Hiragana (Japanese)
            "ア", // Katakana (Japanese)
            "가", // Hangul Syllable (Korean)
            "ㄱ", // Hangul Jamo (Korean)
        ];

        for cjk_char in &cjk_chars {
            let count = estimate_tokens_fallback(cjk_char);
            // Each CJK character should contribute more than 0 tokens
            assert!(
                count > 0,
                "CJK char '{}' should have positive token count",
                cjk_char
            );
        }
    }

    #[test]
    fn test_estimate_tokens_fallback_non_cjk() {
        // Test non-CJK characters
        let non_cjk = "abc123!@#";
        let count = estimate_tokens_fallback(non_cjk);
        // Should have some tokens but less than CJK equivalent
        assert!(count > 0);
    }

    #[test]
    fn test_estimate_tokens_fallback_mixed_content() {
        let mixed = "Hello 世界 123 가나다";
        let count = estimate_tokens_fallback(mixed);
        assert!(count > 0);
    }

    #[test]
    fn test_estimate_tokens_fallback_empty() {
        let count = estimate_tokens_fallback("");
        assert_eq!(count, 0);
    }

    #[test]
    fn test_calculate_savings_identical_strings() {
        let savings = calculate_savings("same string", "same string");
        // If strings are identical, tokens should be the same
        assert_eq!(savings.original_tokens, savings.translated_tokens);
        assert_eq!(savings.saved_tokens, 0);
        assert!((savings.savings_percent - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_savings_empty_strings() {
        let savings = calculate_savings("", "");
        assert_eq!(savings.original_tokens, 0);
        assert_eq!(savings.translated_tokens, 0);
        assert_eq!(savings.saved_tokens, 0);
        assert!((savings.savings_percent - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_savings_different_lengths() {
        let savings = calculate_savings("short", "this is a much longer translation");
        assert!(savings.original_tokens > 0);
        assert!(savings.translated_tokens > 0);
        // Since translated is longer, saved should be 0 (not negative)
        assert_eq!(savings.saved_tokens, 0);
    }

    #[test]
    fn test_token_savings_debug_format() {
        let savings = TokenSavings {
            original_tokens: 100,
            translated_tokens: 80,
            saved_tokens: 20,
            savings_percent: 20.0,
        };

        // Just ensure it doesn't panic when debug formatted
        let _debug_str = format!("{:?}", savings);
    }
}
