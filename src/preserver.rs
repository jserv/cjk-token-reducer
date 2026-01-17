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

// === Term Detector Abstraction ===

/// A detected term with byte offsets
#[derive(Debug, Clone)]
pub struct TermMatch {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Trait for term detection strategies
pub trait TermDetector: Send + Sync {
    fn detect(&self, text: &str) -> Vec<TermMatch>;
}

/// Regex-based detector (all platforms, fallback behavior)
pub struct RegexTermDetector;

impl TermDetector for RegexTermDetector {
    fn detect(&self, text: &str) -> Vec<TermMatch> {
        ENGLISH_TERM_RE
            .find_iter(text)
            .filter(|m| !m.as_str().contains('\u{FEFF}')) // Skip placeholder text
            .map(|m| TermMatch {
                text: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
            })
            .collect()
    }
}

// === macOS NLP Implementation ===

#[cfg(all(target_os = "macos", feature = "macos-nlp"))]
mod macos_nlp {
    use super::*;

    // =========================================================================
    // FFI Module - All unsafe Objective-C interactions are isolated here
    // =========================================================================
    mod ffi {
        use objc2::rc::Retained;
        use objc2::ClassType;
        use objc2_foundation::{NSArray, NSCopying, NSRange, NSString, NSValue};
        use objc2_natural_language::{
            NLTagOrganizationName, NLTagPersonalName, NLTagPlaceName, NLTagSchemeNameType,
            NLTagger, NLTaggerOptions, NLTokenUnit,
        };
        use std::cell::RefCell;

        /// A named entity detected by NLTagger with UTF-16 offsets.
        /// This is the safe output from FFI - no Objective-C types exposed.
        #[derive(Debug, Clone)]
        pub struct NamedEntity {
            pub text: String,
            /// UTF-16 offset (NSString indexing)
            pub utf16_start: usize,
            /// UTF-16 length
            pub utf16_length: usize,
        }

        /// Internal cached tagger state (not exposed outside ffi module).
        struct CachedTagger {
            tagger: Retained<NLTagger>,
            name_type_scheme: Retained<NSString>,
        }

        impl CachedTagger {
            /// Create a new cached tagger. Returns None if tag scheme unavailable.
            fn new() -> Option<Self> {
                // SAFETY: NLTagSchemeNameType is a valid static constant when the
                // NaturalLanguage framework is linked (guaranteed on macOS 10.14+).
                let name_type_scheme = unsafe { NLTagSchemeNameType }?;

                let tag_schemes: Retained<NSArray<NSString>> =
                    NSArray::from_id_slice(&[name_type_scheme.copy()]);

                // SAFETY: NLTagger::alloc returns a valid allocation, and
                // initWithTagSchemes properly initializes it. The tag_schemes
                // array remains valid for the duration of this call.
                let tagger =
                    unsafe { NLTagger::initWithTagSchemes(NLTagger::alloc(), &tag_schemes) };

                Some(CachedTagger {
                    tagger,
                    name_type_scheme: name_type_scheme.copy(),
                })
            }
        }

        // Thread-local cached tagger to avoid allocation overhead per call.
        thread_local! {
            static CACHED_TAGGER: RefCell<Option<CachedTagger>> = const { RefCell::new(None) };
        }

        /// Safe wrapper for NLTagger operations.
        /// All unsafe FFI code is encapsulated within this struct's methods.
        pub struct Tagger;

        impl Tagger {
            /// Detect named entities (PersonalName, PlaceName, OrganizationName) in text.
            /// Returns entities with UTF-16 offsets (caller must convert to UTF-8).
            ///
            /// This is the single safe entry point for NLTagger operations.
            /// All unsafe Objective-C interactions are contained within this method.
            pub fn detect_named_entities(text: &str) -> Vec<NamedEntity> {
                let mut entities = Vec::new();

                // Create NSString from Rust string (safe - objc2 handles encoding)
                let ns_string = NSString::from_str(text);

                CACHED_TAGGER.with(|cache| {
                    let mut cache = cache.borrow_mut();

                    // Lazily initialize the cached tagger
                    if cache.is_none() {
                        *cache = CachedTagger::new();
                    }

                    let Some(cached) = cache.as_ref() else {
                        // Tag scheme unavailable - return empty
                        return;
                    };

                    // SAFETY: tagger and ns_string are valid Retained objects.
                    // ns_string remains valid for the duration of this closure.
                    unsafe { cached.tagger.setString(Some(&ns_string)) };

                    let range = NSRange::new(0, ns_string.length());
                    // NLTaggerOmitOther prevents NSNull in tags array for unrecognized tokens
                    let options = NLTaggerOptions::NLTaggerOmitPunctuation
                        | NLTaggerOptions::NLTaggerOmitWhitespace
                        | NLTaggerOptions::NLTaggerOmitOther
                        | NLTaggerOptions::NLTaggerJoinNames;

                    let mut token_ranges_out: Option<Retained<NSArray<NSValue>>> = None;

                    // SAFETY: All parameters are valid:
                    // - range is within ns_string bounds (0..length)
                    // - name_type_scheme is a valid tag scheme
                    // - token_ranges_out is a valid out-parameter
                    let tags = unsafe {
                        cached.tagger.tagsInRange_unit_scheme_options_tokenRanges(
                            range,
                            NLTokenUnit::Word,
                            &cached.name_type_scheme,
                            options,
                            Some(&mut token_ranges_out),
                        )
                    };

                    // SAFETY: These are valid static constants from the framework.
                    let personal_name = unsafe { NLTagPersonalName };
                    let place_name = unsafe { NLTagPlaceName };
                    let org_name = unsafe { NLTagOrganizationName };

                    if let Some(token_ranges) = token_ranges_out {
                        let count = tags.count().min(token_ranges.count());

                        for idx in 0..count {
                            // SAFETY: idx is within bounds due to min() above.
                            let tag = unsafe { tags.objectAtIndex(idx) };
                            let range_value = unsafe { token_ranges.objectAtIndex(idx) };

                            // Check if this is a named entity type we care about
                            // SAFETY: isEqualToString is safe to call on valid NSString refs.
                            let is_named_entity = personal_name
                                .is_some_and(|pn| unsafe { tag.isEqualToString(pn) })
                                || place_name.is_some_and(|pl| unsafe { tag.isEqualToString(pl) })
                                || org_name.is_some_and(|on| unsafe { tag.isEqualToString(on) });

                            if is_named_entity {
                                // SAFETY: range_value is a valid NSValue containing NSRange.
                                let token_range: NSRange = unsafe { range_value.rangeValue() };

                                // SAFETY: token_range is within ns_string bounds (from tagger).
                                let token_ns_string =
                                    unsafe { ns_string.substringWithRange(token_range) };
                                let token_text = token_ns_string.to_string();

                                entities.push(NamedEntity {
                                    text: token_text,
                                    utf16_start: token_range.location,
                                    utf16_length: token_range.length,
                                });
                            }
                        }
                    }

                    // SAFETY: Setting string to None releases the reference safely.
                    unsafe { cached.tagger.setString(None) };
                });

                entities
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn test_tagger_returns_entities() {
                // Basic smoke test - NLTagger should not panic
                let entities = Tagger::detect_named_entities("Tim Cook works at Apple");
                // We don't assert specific results as NER behavior may vary by OS version
                let _ = entities;
            }

            #[test]
            fn test_tagger_handles_empty_string() {
                let entities = Tagger::detect_named_entities("");
                assert!(entities.is_empty());
            }

            #[test]
            fn test_tagger_handles_no_entities() {
                let entities = Tagger::detect_named_entities("hello world");
                // May or may not find entities - just verify no panic
                let _ = entities;
            }
        }
    }

    // =========================================================================
    // Public API - Safe wrappers using the ffi module
    // =========================================================================

    pub struct MacOsTermDetector;

    impl MacOsTermDetector {
        /// Convert UTF-16 offset to UTF-8 byte offset.
        /// Pure Rust implementation - no unsafe code.
        fn utf16_to_utf8_offset(text: &str, utf16_offset: usize) -> Option<usize> {
            let mut utf16_pos = 0;
            for (byte_idx, ch) in text.char_indices() {
                if utf16_pos == utf16_offset {
                    return Some(byte_idx);
                }
                utf16_pos += ch.len_utf16();
            }
            if utf16_pos == utf16_offset {
                Some(text.len())
            } else {
                None
            }
        }

        /// Check if a character is in the Latin script (including extended Latin).
        /// Covers: Basic Latin, Latin-1 Supplement (letters only), Latin Extended-A/B,
        /// Latin Extended Additional. Excludes math symbols like × ÷.
        /// Pure Rust implementation - no unsafe code.
        fn is_latin_char(c: char) -> bool {
            matches!(
                c,
                // Basic Latin (ASCII letters and digits)
                'A'..='Z' | 'a'..='z' | '0'..='9' |
                // Common punctuation allowed in names
                ' ' | '-' | '\'' | '.' | ',' |
                // Latin-1 Supplement letters (excluding × at U+00D7 and ÷ at U+00F7)
                '\u{00C0}'..='\u{00D6}' |  // À-Ö
                '\u{00D8}'..='\u{00F6}' |  // Ø-ö
                '\u{00F8}'..='\u{00FF}' |  // ø-ÿ
                // Latin Extended-A (Ā-ſ)
                '\u{0100}'..='\u{017F}' |
                // Latin Extended-B (ƀ-ɏ)
                '\u{0180}'..='\u{024F}' |
                // Latin Extended Additional (Ḁ-ỿ, Vietnamese, etc.)
                '\u{1E00}'..='\u{1EFF}'
            )
        }

        /// Check if string contains only Latin script characters.
        /// Allows names like "René", "München", "François" while excluding CJK.
        /// Pure Rust implementation - no unsafe code.
        fn is_latin_only(s: &str) -> bool {
            s.chars().all(Self::is_latin_char)
        }
    }

    impl TermDetector for MacOsTermDetector {
        fn detect(&self, text: &str) -> Vec<TermMatch> {
            // Start with regex-based detection for technical terms
            let mut results = RegexTermDetector.detect(text);

            // Helper to check if a new range overlaps with any existing matches
            let is_overlapping = |start: usize, end: usize, existing: &[TermMatch]| -> bool {
                existing.iter().any(|m| {
                    // Check for intersection: max(start1, start2) < min(end1, end2)
                    start.max(m.start) < end.min(m.end)
                })
            };

            // Use the safe FFI wrapper to detect named entities
            let entities = ffi::Tagger::detect_named_entities(text);

            // Process entities - add NLP-detected named entities that regex missed
            for entity in entities {
                // Skip entities that overlap with existing placeholders (contain FEFF marker)
                // This prevents corruption when NLP runs on text with prior placeholder insertions
                if entity.text.contains('\u{FEFF}') {
                    continue;
                }

                // Only preserve Latin script names (excludes CJK like "张伟")
                // but includes names like "René", "München", "François"
                if Self::is_latin_only(&entity.text) && !entity.text.is_empty() {
                    // Convert UTF-16 offsets to UTF-8 byte offsets
                    if let (Some(start), Some(end)) = (
                        Self::utf16_to_utf8_offset(text, entity.utf16_start),
                        Self::utf16_to_utf8_offset(text, entity.utf16_start + entity.utf16_length),
                    ) {
                        // Only add if not already covered by regex (prevent partial overlaps)
                        if !is_overlapping(start, end, &results) {
                            results.push(TermMatch {
                                text: entity.text,
                                start,
                                end,
                            });
                        }
                    }
                }
            }

            results
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_utf16_to_utf8_offset_ascii() {
            let text = "hello world";
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 0), Some(0));
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 5), Some(5));
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 11), Some(11));
        }

        #[test]
        fn test_utf16_to_utf8_offset_cjk() {
            // Korean: "안녕" (2 chars, 6 UTF-8 bytes, 1 UTF-16 code unit each)
            let text = "안녕";
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 0), Some(0));
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 1), Some(3)); // After first char
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 2), Some(6));
            // End
        }

        #[test]
        fn test_utf16_to_utf8_offset_mixed() {
            // "Hi안녕" - 2 ASCII + 2 Korean
            let text = "Hi안녕";
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 0), Some(0)); // H
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 2), Some(2)); // After "Hi"
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 3), Some(5)); // After "Hi안"
            assert_eq!(MacOsTermDetector::utf16_to_utf8_offset(text, 4), Some(8));
            // End
        }

        #[test]
        fn test_is_latin_only() {
            // Basic ASCII
            assert!(MacOsTermDetector::is_latin_only("hello"));
            assert!(MacOsTermDetector::is_latin_only("Tim Cook"));
            assert!(MacOsTermDetector::is_latin_only("Apple123"));
            // Latin extended characters (accents, umlauts)
            assert!(MacOsTermDetector::is_latin_only("René"));
            assert!(MacOsTermDetector::is_latin_only("München"));
            assert!(MacOsTermDetector::is_latin_only("José García"));
            assert!(MacOsTermDetector::is_latin_only("Zürich"));
            assert!(MacOsTermDetector::is_latin_only("Ångström"));
            // CJK should be rejected
            assert!(!MacOsTermDetector::is_latin_only("张伟"));
            assert!(!MacOsTermDetector::is_latin_only("Tim张"));
            assert!(!MacOsTermDetector::is_latin_only("東京"));
            assert!(!MacOsTermDetector::is_latin_only("서울"));
            // Math symbols should be rejected (× = U+00D7, ÷ = U+00F7)
            assert!(!MacOsTermDetector::is_latin_only("3×4"));
            assert!(!MacOsTermDetector::is_latin_only("8÷2"));
        }

        #[test]
        fn test_macos_detector_basic() {
            let detector = MacOsTermDetector;
            // This tests that the detector can be instantiated and called
            // Actual NER results depend on the macOS NLP model
            let _matches = detector.detect("Tim Cook works at Apple");
            // We don't assert specific results as NER behavior may vary
        }

        #[test]
        fn test_macos_detector_filters_cjk_names() {
            let detector = MacOsTermDetector;
            let matches = detector.detect("张伟 works at 苹果公司");
            // Should NOT preserve Chinese names (filtered by is_latin_only)
            assert!(!matches.iter().any(|m| m.text.contains('张')));
            assert!(!matches.iter().any(|m| m.text.contains('苹')));
        }
    }
}

/// Get the appropriate term detector for the platform and configuration
#[allow(unused_variables)]
pub fn get_term_detector(use_nlp: bool) -> Box<dyn TermDetector> {
    #[cfg(all(target_os = "macos", feature = "macos-nlp"))]
    if use_nlp {
        return Box::new(macos_nlp::MacOsTermDetector);
    }

    Box::new(RegexTermDetector)
}

/// Configuration for preservation behavior
#[derive(Debug, Clone, Default)]
pub struct PreserveConfig {
    /// Enable [[...]] wiki-style markers
    pub wiki_markers: bool,
    /// Enable ==...== highlight-style markers
    pub highlight_markers: bool,
    /// Enable auto-detection of English technical terms in CJK text
    pub english_terms: bool,
    /// Use macOS NLP for term detection (macOS only, falls back to regex)
    pub use_nlp: bool,
}

impl PreserveConfig {
    /// Default config: all preservation features enabled
    pub fn all() -> Self {
        Self {
            wiki_markers: true,
            highlight_markers: true,
            english_terms: true,
            use_nlp: true, // Enable NLP by default on macOS
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
    // Uses either macOS NLP (if enabled and available) or regex fallback
    if config.english_terms {
        let detector = get_term_detector(config.use_nlp);
        let mut terms = detector.detect(&result);

        // Sort by start position descending to process in reverse order
        // This preserves byte indices during replacement
        terms.sort_by(|a, b| b.start.cmp(&a.start));

        for term in terms {
            let placeholder = format!("\u{FEFF}cjkengterm{index}\u{FEFF}");
            segments.push(PreservedSegment {
                placeholder: placeholder.clone(),
                original: term.text,
                segment_type: SegmentType::EnglishTerm,
            });
            result.replace_range(term.start..term.end, &placeholder);
            index += 1;
        }
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

    #[test]
    fn test_extract_preserve_restore_roundtrip_simple() {
        let text = "이 함수 `foo()`를 호출하세요";
        let preserved = extract_and_preserve(text);
        let restored = restore_preserved(&preserved.text, &preserved.segments);

        assert_eq!(restored, text);
    }

    #[test]
    fn test_extract_preserve_restore_roundtrip_complex() {
        let text = "이 코드 수정해줘\n```rust\nfn main() {}\n```\nAPI_KEY 환경변수 필요";
        let preserved = extract_and_preserve(text);
        let restored = restore_preserved(&preserved.text, &preserved.segments);

        assert_eq!(restored, text);
    }

    #[test]
    fn test_extract_preserve_restore_roundtrip_all_types() {
        // Wiki markers [[...]] are intentionally stripped during extraction (capture group)
        // The expected output keeps the inner content but removes the markers
        let text = "[[keep]] `code` getUserData API ./src/main.rs https://example.com";
        let expected = "keep `code` getUserData API ./src/main.rs https://example.com";
        let config = PreserveConfig::all();
        let preserved = extract_and_preserve_with_config(text, &config);
        let restored = restore_preserved(&preserved.text, &preserved.segments);

        assert_eq!(restored, expected);
    }

    #[test]
    fn test_extract_preserve_empty_text() {
        let text = "";
        let preserved = extract_and_preserve(text);

        assert_eq!(preserved.text, "");
        assert!(preserved.segments.is_empty());
    }

    #[test]
    fn test_extract_preserve_no_segments() {
        let text = "이 텍스트는 보호할 세그먼트가 없습니다";
        let preserved = extract_and_preserve(text);

        // No segments should be extracted
        assert!(preserved.segments.is_empty());
        assert_eq!(preserved.text, text);
    }

    #[test]
    fn test_regex_term_detector_filters_placeholders() {
        // Ensure regex detector doesn't match placeholder text itself
        let text = "\u{FEFF}cjkengterm0\u{FEFF} should not be matched";
        let detector = RegexTermDetector;
        let matches = detector.detect(text);

        // Should not match the placeholder
        assert!(!matches.iter().any(|m| m.text.contains("cjkengterm")));
    }

    #[test]
    fn test_term_match_properties() {
        let text = "getUserData 함수";
        let detector = RegexTermDetector;
        let matches = detector.detect(text);

        assert!(!matches.is_empty());
        let term = &matches[0];
        assert_eq!(term.text, "getUserData");
        assert_eq!(term.start, 0);
        // End position should be after "getUserData"
        assert!(term.end > 0);
    }
}
