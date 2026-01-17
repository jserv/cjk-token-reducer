use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct PreservedSegment {
    pub placeholder: String,
    pub original: String,
    pub segment_type: SegmentType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SegmentType {
    CodeBlock,
    InlineCode,
    Url,
    FilePath,
    NoTranslate, // User-marked text [[...]] or ==...==
    EnglishTerm, // Auto-detected English technical terms in CJK text
}

pub struct PreserveResult {
    pub text: String,
    pub segments: Vec<PreservedSegment>,
}

// Lazy-compiled regexes (compiled once, reused)
static CODE_BLOCK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"```[\s\S]*?```").unwrap());
static INLINE_CODE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`[^`]+`").unwrap());
// Exclude trailing punctuation from URLs
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://[^\s]*[^\s.,;)]").unwrap());
static FILE_PATH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:\.\.?/)?(?:[\w.\-]+/)+[\w.\-]+(?:\.\w+)?").unwrap());

// No-translate markers: [[text]] and ==text==
static WIKI_MARKER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[\[([^\]]+)\]\]").unwrap());
static HIGHLIGHT_MARKER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"==([^=]+)==").unwrap());

// English technical terms: camelCase, PascalCase, SCREAMING_CASE, snake_case identifiers
// Matches: getUserData, API_KEY, MyClass, fetch_results, MAX_SIZE, getURLData, XMLParser
static ENGLISH_TERM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)
        # camelCase with acronyms: getURLData, parseXMLFile, myHTTPClient
        [a-z]+(?:[A-Z]+[a-z]*)+  |
        # PascalCase with acronyms: XMLParser, HTTPRequest, MyURLHandler
        [A-Z]+[a-z]+(?:[A-Z]+[a-z]*)*  |
        # Acronym + PascalCase: URLParser, HTTPClient, XMLHttpRequest
        [A-Z]{2,}[a-z]+(?:[A-Z]+[a-z]*)*  |
        # SCREAMING_SNAKE_CASE (2+ parts)
        [A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+ |
        # snake_case identifiers (2+ parts)
        [a-z][a-z0-9]*(?:_[a-z0-9]+)+ |
        # Common acronyms and short tech terms (standalone, 3+ chars)
        \b(?:API|URL|HTTP|JSON|XML|SQL|CSS|HTML|DOM|SDK|CLI|GUI|IDE|ORM|MVC|MVP|REST|CRUD|AJAX|UUID|UTF|ASCII|JPEG|PNG|GIF|PDF|ZIP|SSH|SSL|TLS|TCP|UDP|DNS|FTP|SMTP|IMAP|POP3|LDAP|OAuth|JWT|CORS|CSRF|XSS|CDN|AWS|GCP|VPN|IoT|CPU|GPU|RAM|SSD|HDD|USB|BIOS|UEFI|EFI|NAS|RAID)\b
    ").unwrap()
});

/// Configuration for preservation behavior
#[derive(Debug, Clone, Default)]
pub struct PreserveConfig {
    /// Enable [[...]] wiki-style markers
    pub wiki_markers: bool,
    /// Enable ==...== highlight-style markers
    pub highlight_markers: bool,
    /// Enable auto-detection of English technical terms in CJK text
    pub english_terms: bool,
}

impl PreserveConfig {
    /// Default config: all preservation features enabled
    pub fn all() -> Self {
        Self {
            wiki_markers: true,
            highlight_markers: true,
            english_terms: true,
        }
    }

    /// Config with only basic preservation (code, URLs, paths)
    pub fn basic() -> Self {
        Self::default()
    }
}

/// Get the type string for a segment type (used in placeholder generation)
fn segment_type_str(segment_type: SegmentType) -> &'static str {
    match segment_type {
        SegmentType::CodeBlock => "code",
        SegmentType::InlineCode => "inline",
        SegmentType::Url => "url",
        SegmentType::FilePath => "path",
        SegmentType::NoTranslate => "notrans",
        SegmentType::EnglishTerm => "engterm",
    }
}

/// Replace regex matches with placeholders, collecting preserved segments.
/// If `use_capture_group` is true, stores only capture group 1 (for markers like [[text]]).
/// Otherwise stores the full match.
fn replace_with_placeholders(
    text: &str,
    regex: &Regex,
    segment_type: SegmentType,
    segments: &mut Vec<PreservedSegment>,
    index: &mut usize,
    use_capture_group: bool,
) -> String {
    let type_str = segment_type_str(segment_type);
    regex
        .replace_all(text, |caps: &regex::Captures| {
            let original = if use_capture_group {
                caps.get(1)
                    .map(|m| m.as_str())
                    .unwrap_or(&caps[0])
                    .to_string()
            } else {
                caps[0].to_string()
            };
            let placeholder = format!("\u{FEFF}cjk{type_str}{index}\u{FEFF}");
            segments.push(PreservedSegment {
                placeholder: placeholder.clone(),
                original,
                segment_type,
            });
            *index += 1;
            placeholder
        })
        .into_owned()
}

/// Extract code blocks, inline code, URLs, and file paths, replacing with placeholders
/// Uses default config (basic preservation only)
pub fn extract_and_preserve(text: &str) -> PreserveResult {
    extract_and_preserve_with_config(text, &PreserveConfig::default())
}

/// Extract and preserve with configurable options
pub fn extract_and_preserve_with_config(text: &str, config: &PreserveConfig) -> PreserveResult {
    let mut segments = Vec::new();
    let mut index = 0;

    // Priority order: code blocks > inline code > no-translate markers > URLs > file paths > English terms
    // Higher priority patterns are extracted first to prevent overlap

    // 1. Code blocks (highest priority - multiline)
    let mut result = replace_with_placeholders(
        text,
        &CODE_BLOCK_RE,
        SegmentType::CodeBlock,
        &mut segments,
        &mut index,
        false,
    );

    // 2. Inline code
    result = replace_with_placeholders(
        &result,
        &INLINE_CODE_RE,
        SegmentType::InlineCode,
        &mut segments,
        &mut index,
        false,
    );

    // 3. No-translate markers [[...]] (wiki-style) - uses capture group for inner content
    if config.wiki_markers {
        result = replace_with_placeholders(
            &result,
            &WIKI_MARKER_RE,
            SegmentType::NoTranslate,
            &mut segments,
            &mut index,
            true,
        );
    }

    // 4. No-translate markers ==...== (highlight-style) - uses capture group for inner content
    if config.highlight_markers {
        result = replace_with_placeholders(
            &result,
            &HIGHLIGHT_MARKER_RE,
            SegmentType::NoTranslate,
            &mut segments,
            &mut index,
            true,
        );
    }

    // 5. URLs
    result = replace_with_placeholders(
        &result,
        &URL_RE,
        SegmentType::Url,
        &mut segments,
        &mut index,
        false,
    );

    // 6. File paths
    result = replace_with_placeholders(
        &result,
        &FILE_PATH_RE,
        SegmentType::FilePath,
        &mut segments,
        &mut index,
        false,
    );

    // 7. English technical terms (lowest priority - only in remaining text)
    if config.english_terms {
        result = replace_with_placeholders(
            &result,
            &ENGLISH_TERM_RE,
            SegmentType::EnglishTerm,
            &mut segments,
            &mut index,
            false,
        );
    }

    PreserveResult {
        text: result,
        segments,
    }
}

/// Restore preserved segments back to original text
pub fn restore_preserved(text: &str, segments: &[PreservedSegment]) -> String {
    let mut result = text.to_string();
    // Restore in reverse order to avoid collisions where a restored segment
    // contains text that looks like a later placeholder.
    for segment in segments.iter().rev() {
        result = result.replace(&segment.placeholder, &segment.original);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_block_preservation() {
        let text = "이 코드 수정해줘\n```rust\nfn main() {}
```";
        let result = extract_and_preserve(text);
        assert_eq!(result.segments.len(), 1);
        assert!(result.text.contains("cjkcode"));
        assert!(result.segments[0].original.contains("fn main()"));
    }

    #[test]
    fn test_inline_code_preservation() {
        let text = "함수 `foo()` 호출해줘";
        let result = extract_and_preserve(text);
        assert_eq!(result.segments.len(), 1);
        assert!(result.text.contains("cjkinline"));
        assert_eq!(result.segments[0].original, "`foo()`");
    }

    #[test]
    fn test_url_preservation() {
        let text = "https://github.com/foo 이 링크 참고해";
        let result = extract_and_preserve(text);
        assert!(result
            .segments
            .iter()
            .any(|s| s.original.contains("github.com")));
    }

    #[test]
    fn test_url_punctuation_exclusion() {
        let text = "Check https://github.com/foo.";
        let result = extract_and_preserve(text);
        let segment = result
            .segments
            .iter()
            .find(|s| matches!(s.segment_type, SegmentType::Url))
            .unwrap();
        assert_eq!(segment.original, "https://github.com/foo"); // Dot should not be included
        assert!(result.text.ends_with(".")); // Dot should remain in text
    }

    #[test]
    fn test_file_path_preservation() {
        let text = "./src/main.rs 파일 수정해줘";
        let result = extract_and_preserve(text);
        assert!(result
            .segments
            .iter()
            .any(|s| s.original.contains("src/main.rs")));
    }

    #[test]
    fn test_restore_order() {
        let text = "코드 `foo()` 수정 ```\nbar()
```";
        let preserved = extract_and_preserve(text);
        let restored = restore_preserved(&preserved.text, &preserved.segments);
        assert!(restored.contains("`foo()`"));
        assert!(restored.contains(
            "```\nbar()
```"
        ));
    }

    #[test]
    fn test_restore_collision() {
        // This tests that if a code block contains text that mimics a generated placeholder,
        // it doesn't get double-replaced.
        // We artificially construct a case where the user text predicts the next placeholder.
        // "Code: `__PRESERVE_URL_1__` Link: https://example.com"
        // 1. Inline matches `__PRESERVE_URL_1__` -> becomes __PRESERVE_INLINE_0__
        // 2. URL matches https://example.com -> becomes __PRESERVE_URL_1__
        // If we restore forward:
        // __PRESERVE_INLINE_0__ -> `__PRESERVE_URL_1__`
        // Then `__PRESERVE_URL_1__` -> https://example.com (WRONG)
        let text = "Code: `__PRESERVE_URL_1__` Link: https://example.com";
        let preserved = extract_and_preserve(text);
        let restored = restore_preserved(&preserved.text, &preserved.segments);
        assert_eq!(restored, text);
    }

    // === No-Translate Marker Tests ===

    #[test]
    fn test_wiki_marker_preservation() {
        let text = "이 함수는 [[getUserData]]를 호출합니다";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        // Should have one NoTranslate segment
        let no_trans: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::NoTranslate)
            .collect();
        assert_eq!(
            no_trans.len(),
            1,
            "Expected 1 NoTranslate segment, got {}",
            no_trans.len()
        );
        // Original should be inner content without markers
        assert_eq!(no_trans[0].original, "getUserData");
        // Text should have placeholder, not markers
        assert!(
            result.text.contains("cjknotrans"),
            "Expected notrans placeholder in: {}",
            result.text
        );
        assert!(!result.text.contains("[["));
    }

    #[test]
    fn test_highlight_marker_preservation() {
        let text = "==API_KEY== 환경변수를 설정하세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let no_trans: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::NoTranslate)
            .collect();
        assert_eq!(no_trans.len(), 1);
        assert_eq!(no_trans[0].original, "API_KEY");
        assert!(!result.text.contains("=="));
    }

    #[test]
    fn test_multiple_markers() {
        let text = "[[foo]] and ==bar== and [[baz]]";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let no_trans: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::NoTranslate)
            .collect();
        assert_eq!(no_trans.len(), 3);
        assert!(no_trans.iter().any(|s| s.original == "foo"));
        assert!(no_trans.iter().any(|s| s.original == "bar"));
        assert!(no_trans.iter().any(|s| s.original == "baz"));
    }

    #[test]
    fn test_marker_restore() {
        let text = "이 함수는 [[getUserData]]를 호출합니다";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);
        let restored = restore_preserved(&result.text, &result.segments);
        // Restored should have inner content (markers stripped)
        assert!(restored.contains("getUserData"));
        // But not the markers themselves
        assert!(!restored.contains("[["));
        assert!(!restored.contains("]]"));
    }

    #[test]
    fn test_markers_disabled() {
        let text = "[[keep]] and ==this==";
        let config = PreserveConfig::basic(); // All disabled
        let result = extract_and_preserve_with_config(text, &config);

        // No NoTranslate segments
        let no_trans: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::NoTranslate)
            .collect();
        assert_eq!(no_trans.len(), 0);
        // Markers should remain in text
        assert!(result.text.contains("[[keep]]"));
        assert!(result.text.contains("==this=="));
    }

    // === English Technical Term Tests ===

    #[test]
    fn test_camel_case_detection() {
        let text = "getUserData 함수를 호출해주세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "getUserData"));
    }

    #[test]
    fn test_pascal_case_detection() {
        let text = "MyClass 클래스를 사용하세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "MyClass"));
    }

    #[test]
    fn test_screaming_snake_case_detection() {
        let text = "MAX_SIZE 상수를 변경하세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "MAX_SIZE"));
    }

    #[test]
    fn test_snake_case_detection() {
        let text = "get_user_data 함수입니다";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "get_user_data"));
    }

    #[test]
    fn test_common_acronyms() {
        let text = "API 요청을 보내세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "API"));
    }

    #[test]
    fn test_english_terms_disabled() {
        let text = "getUserData 함수";
        let mut config = PreserveConfig::all();
        config.english_terms = false;
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert_eq!(eng_terms.len(), 0);
    }

    #[test]
    fn test_mixed_preservation() {
        let text = "[[keep]] `code` getUserData API 파일 ./src/main.rs";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        // Check all types are preserved
        assert!(result
            .segments
            .iter()
            .any(|s| s.segment_type == SegmentType::NoTranslate));
        assert!(result
            .segments
            .iter()
            .any(|s| s.segment_type == SegmentType::InlineCode));
        assert!(result
            .segments
            .iter()
            .any(|s| s.segment_type == SegmentType::EnglishTerm));
        assert!(result
            .segments
            .iter()
            .any(|s| s.segment_type == SegmentType::FilePath));
    }

    // === Acronym-in-identifier Tests (Codex review findings) ===

    #[test]
    fn test_acronym_in_camelcase() {
        let text = "getURLData parseXMLFile myHTTPClient 함수";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "getURLData"));
        assert!(eng_terms.iter().any(|s| s.original == "parseXMLFile"));
        assert!(eng_terms.iter().any(|s| s.original == "myHTTPClient"));
    }

    #[test]
    fn test_acronym_leading_pascalcase() {
        let text = "XMLParser HTTPRequest URLHandler 클래스";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "XMLParser"));
        assert!(eng_terms.iter().any(|s| s.original == "HTTPRequest"));
        assert!(eng_terms.iter().any(|s| s.original == "URLHandler"));
    }

    #[test]
    fn test_xmlhttprequest_style() {
        let text = "XMLHttpRequest 객체를 사용하세요";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert!(eng_terms.iter().any(|s| s.original == "XMLHttpRequest"));
    }

    // === Edge case tests ===

    #[test]
    fn test_single_word_not_preserved() {
        // Single lowercase words should NOT be preserved (avoid preserving common English)
        let text = "hello world 안녕";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        // "hello" and "world" should not be preserved
        assert!(!eng_terms.iter().any(|s| s.original == "hello"));
        assert!(!eng_terms.iter().any(|s| s.original == "world"));
    }

    #[test]
    fn test_placeholder_no_collision() {
        // Ensure placeholders don't get re-matched as English terms
        let text = "[[keep]] 테스트";
        let config = PreserveConfig::all();
        let result = extract_and_preserve_with_config(text, &config);

        // Should only have 1 NoTranslate segment, no EnglishTerm matching placeholder
        let no_trans: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::NoTranslate)
            .collect();
        let eng_terms: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.segment_type == SegmentType::EnglishTerm)
            .collect();
        assert_eq!(no_trans.len(), 1);
        assert_eq!(eng_terms.len(), 0);
    }
}
