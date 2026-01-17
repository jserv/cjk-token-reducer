//! Security utilities for preventing sensitive data leakage
//!
//! This module provides functions to:
//! - Sanitize prompt contents in error messages and logs
//! - Redact API keys and credentials
//! - Truncate sensitive data for safe display
//!
//! Security principle: Never log API keys or full prompt contents.

use once_cell::sync::Lazy;
use regex::Regex;
use std::borrow::Cow;

/// Maximum length for prompt content in error messages/logs
const MAX_PROMPT_PREVIEW_LEN: usize = 50;

/// Patterns that indicate potential API keys or secrets
const SECRET_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "secret",
    "password",
    "token",
    "bearer",
    "authorization",
    "credential",
    "private_key",
    "access_key",
];

/// Pre-compiled regex patterns for secret redaction (compiled once at startup)
static REDACTION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    let mut patterns = Vec::new();
    for pattern in SECRET_PATTERNS {
        // Match key=value or key: value (handles "Bearer <token>" style)
        // Group 1: key and separator (including optional Bearer prefix)
        // Group 2: the actual secret value
        if let Ok(re) = Regex::new(&format!(
            r#"(?i)({}\s*[:=]\s*(?:Bearer\s+)?)([^\s"',}}\]]+)"#,
            regex::escape(pattern)
        )) {
            patterns.push(re);
        }
        // Match JSON style: "key": "value"
        if let Ok(re) = Regex::new(&format!(
            r#"(?i)("{}":\s*)"([^"]+)""#,
            regex::escape(pattern)
        )) {
            patterns.push(re);
        }
    }
    patterns
});

/// Sanitize text for safe inclusion in error messages or logs.
///
/// This function:
/// - Truncates long text to prevent prompt content leakage
/// - Replaces newlines with escaped representation
/// - Adds ellipsis indicator when truncated
///
/// Performance: For large inputs, truncates first to avoid processing megabytes of data.
///
/// # Examples
/// ```
/// use cjk_token_reducer::security::sanitize_for_log;
///
/// let long_text = "This is a very long prompt with sensitive information...";
/// let safe = sanitize_for_log(long_text, 20);
/// assert!(safe.len() <= 23); // 20 chars + "..."
/// assert!(safe.ends_with("..."));
/// ```
pub fn sanitize_for_log(text: &str, max_len: usize) -> Cow<'_, str> {
    // Handle empty input
    if text.is_empty() {
        return Cow::Borrowed(text);
    }

    // Optimization: For large inputs, pre-truncate to avoid processing megabytes
    // Use 2x max_len as buffer for escape expansion (each char can become 2 chars max)
    let limit = max_len.saturating_mul(2).max(100);
    let slice = if text.len() > limit {
        let mut end = limit;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };

    // Replace control characters for safe display
    let needs_escape = slice.contains('\n') || slice.contains('\r') || slice.contains('\t');

    if slice.len() <= max_len && !needs_escape {
        return Cow::Borrowed(slice);
    }

    let escaped = if needs_escape {
        slice
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    } else {
        slice.to_string()
    };

    if escaped.len() <= max_len {
        return Cow::Owned(escaped);
    }

    // Find char boundary for truncation
    let mut truncate_at = max_len;
    while truncate_at > 0 && !escaped.is_char_boundary(truncate_at) {
        truncate_at -= 1;
    }

    Cow::Owned(format!("{}...", &escaped[..truncate_at]))
}

/// Sanitize text for error messages (default truncation)
pub fn sanitize_for_error(text: &str) -> Cow<'_, str> {
    sanitize_for_log(text, MAX_PROMPT_PREVIEW_LEN)
}

/// Check if a string looks like it might contain an API key or secret
///
/// Returns true if the string contains patterns commonly associated with secrets.
/// Used to add extra warnings when handling potentially sensitive data.
pub fn looks_like_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    SECRET_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Redact potential secrets from a string for safe logging
///
/// Replaces values that look like API keys or tokens with "[REDACTED]"
/// Uses pre-compiled regex patterns for performance.
pub fn redact_secrets(text: &str) -> String {
    let mut result = text.to_string();

    // Use pre-compiled patterns for efficiency
    for re in REDACTION_PATTERNS.iter() {
        result = re.replace_all(&result, "${1}[REDACTED]").to_string();
    }

    result
}

/// Format a prompt preview for debug output
///
/// Shows length and a truncated preview without exposing full content.
pub fn format_prompt_preview(prompt: &str) -> String {
    let char_count = prompt.chars().count();
    let preview = sanitize_for_log(prompt, 30);
    format!("[{} chars]: {}", char_count, preview)
}

/// Warning message for debug commands that may expose sensitive data
pub const SENSITIVE_DATA_WARNING: &str =
    "WARNING: Debug output may contain sensitive prompt contents. Do not share in public logs.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_for_log_short() {
        let text = "short";
        assert_eq!(sanitize_for_log(text, 50).as_ref(), "short");
    }

    #[test]
    fn test_sanitize_for_log_truncates() {
        let text = "this is a long text that should be truncated";
        let result = sanitize_for_log(text, 10);
        assert_eq!(result.as_ref(), "this is a ...");
    }

    #[test]
    fn test_sanitize_for_log_escapes_newlines() {
        let text = "line1\nline2";
        let result = sanitize_for_log(text, 100);
        assert_eq!(result.as_ref(), "line1\\nline2");
    }

    #[test]
    fn test_sanitize_for_log_escapes_tabs() {
        let text = "col1\tcol2";
        let result = sanitize_for_log(text, 100);
        assert_eq!(result.as_ref(), "col1\\tcol2");
    }

    #[test]
    fn test_sanitize_for_log_unicode() {
        let text = "你好世界";
        let result = sanitize_for_log(text, 2);
        // Should not panic on char boundary
        assert!(result.ends_with("...") || result.len() <= 6);
    }

    #[test]
    fn test_sanitize_for_error() {
        let text = "a".repeat(100);
        let result = sanitize_for_error(&text);
        assert!(result.len() <= MAX_PROMPT_PREVIEW_LEN + 3);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_looks_like_secret() {
        assert!(looks_like_secret("api_key=abc123"));
        assert!(looks_like_secret("my secret password"));
        assert!(looks_like_secret("Bearer token here"));
        assert!(!looks_like_secret("normal text"));
        assert!(!looks_like_secret("hello world"));
    }

    #[test]
    fn test_redact_secrets() {
        let input = "api_key=sk-12345 and password: hunter2";
        let result = redact_secrets(input);
        assert!(
            !result.contains("sk-12345"),
            "sk-12345 should be redacted: {}",
            result
        );
        assert!(
            !result.contains("hunter2"),
            "hunter2 should be redacted: {}",
            result
        );
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_redact_secrets_json() {
        let input = r#"{"api_key": "secret123", "data": "safe"}"#;
        let result = redact_secrets(input);
        assert!(
            !result.contains("secret123"),
            "secret123 should be redacted: {}",
            result
        );
        assert!(result.contains("safe"));
    }

    #[test]
    fn test_redact_bearer_token() {
        // Test Authorization: Bearer <token> pattern
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let result = redact_secrets(input);
        assert!(
            !result.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
            "Bearer token should be redacted: {}",
            result
        );
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_redact_token_header() {
        // Test token= pattern
        let input = "token=abc123xyz";
        let result = redact_secrets(input);
        assert!(
            !result.contains("abc123xyz"),
            "Token should be redacted: {}",
            result
        );
    }

    #[test]
    fn test_format_prompt_preview() {
        let prompt = "This is a test prompt with some content";
        let preview = format_prompt_preview(prompt);
        // 39 chars in the prompt
        assert!(
            preview.contains("39 chars"),
            "Expected '39 chars' in: {}",
            preview
        );
        // With max 30 char preview, it should be truncated
        assert!(preview.contains("..."), "Expected '...' in: {}", preview);
    }

    #[test]
    fn test_sanitize_empty() {
        assert_eq!(sanitize_for_log("", 50).as_ref(), "");
        assert_eq!(sanitize_for_error("").as_ref(), "");
    }

    #[test]
    fn test_sanitize_large_input() {
        // Test that large inputs are handled efficiently (pre-truncated)
        let large_text = "a".repeat(1_000_000); // 1MB of 'a'
        let result = sanitize_for_log(&large_text, 50);
        assert!(result.len() <= 53); // 50 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_sanitize_large_input_with_newlines() {
        // Large input with newlines - should still be efficient
        let large_text = "line\n".repeat(100_000); // Many lines
        let result = sanitize_for_log(&large_text, 20);
        assert!(result.len() <= 23); // 20 + "..."
        assert!(result.contains("\\n")); // Newlines should be escaped
    }
}
