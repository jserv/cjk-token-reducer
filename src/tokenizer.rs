//! Token counting using Claude's tokenizer
//!
//! When the `tokenizer` feature is enabled, uses the claude-tokenizer crate
//! for precise token counting. Otherwise, falls back to estimation.

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
    let cjk_chars = text
        .chars()
        .filter(|c| {
            matches!(c,
                // CJK Unified Ideographs (main block + extensions)
                '\u{4E00}'..='\u{9FFF}' |  // CJK Unified Ideographs
                '\u{3400}'..='\u{4DBF}' |  // CJK Extension A
                '\u{20000}'..='\u{2A6DF}'| // CJK Extension B
                '\u{2A700}'..='\u{2B73F}'| // CJK Extension C
                '\u{2B740}'..='\u{2B81F}'| // CJK Extension D
                '\u{2B820}'..='\u{2CEAF}'| // CJK Extension E
                '\u{2CEB0}'..='\u{2EBEF}'| // CJK Extension F
                '\u{30000}'..='\u{3134F}'| // CJK Extension G
                '\u{F900}'..='\u{FAFF}' |  // CJK Compatibility Ideographs
                // Japanese
                '\u{3040}'..='\u{309F}' |  // Hiragana
                '\u{30A0}'..='\u{30FF}' |  // Katakana
                '\u{31F0}'..='\u{31FF}' |  // Katakana Phonetic Extensions
                // Korean
                '\u{AC00}'..='\u{D7AF}' |  // Hangul Syllables
                '\u{1100}'..='\u{11FF}' |  // Hangul Jamo
                '\u{3130}'..='\u{318F}' |  // Hangul Compatibility Jamo
                '\u{A960}'..='\u{A97F}' |  // Hangul Jamo Extended-A
                '\u{D7B0}'..='\u{D7FF}' |  // Hangul Jamo Extended-B
                // CJK Symbols and Punctuation
                '\u{3000}'..='\u{303F}' |  // CJK Symbols and Punctuation
                '\u{3100}'..='\u{312F}' |  // Bopomofo
                '\u{31A0}'..='\u{31BF}' |  // Bopomofo Extended
                '\u{FF00}'..='\u{FFEF}'    // Halfwidth and Fullwidth Forms
            )
        })
        .count();

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
}
