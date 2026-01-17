#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Chinese,
    Japanese,
    Korean,
    English,
    Unknown,
}

impl Language {
    pub fn code(&self) -> &'static str {
        match self {
            Language::Chinese => "zh-TW",
            Language::Japanese => "ja",
            Language::Korean => "ko",
            Language::English => "en",
            Language::Unknown => "auto",
        }
    }
}

#[derive(Debug)]
pub struct DetectionResult {
    pub language: Language,
    pub ratio: f64,
}

#[derive(Debug, Default)]
struct CharCounts {
    chinese: usize,
    japanese: usize,
    korean: usize,
    total: usize,
}

/// Detect the dominant CJK language in text
pub fn detect_language(text: &str) -> DetectionResult {
    let mut counts = CharCounts::default();

    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        counts.total += 1;

        match ch {
            // CJK Unified Ideographs (Chinese + Japanese Kanji)
            '\u{4E00}'..='\u{9FFF}' => counts.chinese += 1,

            // Japanese-specific: Hiragana, Katakana
            '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' => counts.japanese += 1,

            // Korean: Hangul Syllables, Jamo, Compatibility Jamo
            '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' => {
                counts.korean += 1
            }

            _ => {}
        }
    }

    // Determine dominant language
    // Japanese text typically mixes Kanji with Kana, so we weight it
    let cjk_scores = [
        (Language::Chinese, counts.chinese),
        (Language::Japanese, counts.japanese + counts.chinese / 3),
        (Language::Korean, counts.korean),
    ];

    let (language, count) = cjk_scores
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .unwrap_or((Language::English, 0));

    let cjk_total = counts.korean + counts.japanese + counts.chinese;
    let ratio = if counts.total > 0 {
        cjk_total as f64 / counts.total as f64
    } else {
        0.0
    };

    let language = if count == 0 {
        Language::English
    } else {
        language
    };

    DetectionResult { language, ratio }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chinese_detection() {
        let result = detect_language("請重構這個函式");
        assert_eq!(result.language, Language::Chinese);
        assert!(result.ratio > 0.5);
    }

    #[test]
    fn test_japanese_detection() {
        let result = detect_language("この関数をリファクタリングしてください");
        assert_eq!(result.language, Language::Japanese);
        assert!(result.ratio > 0.5);
    }

    #[test]
    fn test_korean_detection() {
        let result = detect_language("이 함수 리팩토링 해줘");
        assert_eq!(result.language, Language::Korean);
        assert!(result.ratio > 0.5);
    }

    #[test]
    fn test_english_detection() {
        let result = detect_language("Refactor this function please");
        assert_eq!(result.language, Language::English);
        assert!(result.ratio < 0.1);
    }

    #[test]
    fn test_mixed_content() {
        let result = detect_language("function foo() { } // 이 함수는 버그가 있음");
        assert!(result.ratio > 0.1);
    }

    #[test]
    fn test_language_priority_chinese_over_english() {
        // Chinese should be detected even with English characters
        let result = detect_language("這個function需要refactor");
        assert_eq!(result.language, Language::Chinese);
        // Just verify Chinese is detected (ratio depends on specific character counts)
        assert!(result.ratio > 0.0);
    }

    #[test]
    fn test_language_priority_japanese_with_kanji() {
        // Japanese with Kanji should prioritize Japanese over Chinese
        let result = detect_language("この関数をリファクタリングしてください");
        assert_eq!(result.language, Language::Japanese);
        assert!(result.ratio > 0.5);
    }

    #[test]
    fn test_language_priority_japanese_mixed() {
        // Japanese with Kanji + Kana - should still be Japanese
        let result = detect_language("漢字とひらがな");
        assert_eq!(result.language, Language::Japanese);
    }

    #[test]
    fn test_language_priority_korean() {
        // Pure Korean should be detected
        let result = detect_language("이 함수를 수정해주세요");
        assert_eq!(result.language, Language::Korean);
        assert!(result.ratio > 0.8);
    }

    #[test]
    fn test_empty_string() {
        let result = detect_language("");
        assert_eq!(result.language, Language::English);
        assert_eq!(result.ratio, 0.0);
    }

    #[test]
    fn test_whitespace_only() {
        let result = detect_language("   \n\t  ");
        assert_eq!(result.language, Language::English);
        assert_eq!(result.ratio, 0.0);
    }

    #[test]
    fn test_japanese_weighting() {
        // Japanese text typically mixes Kanji (Chinese range) with Kana
        // The detector should weight Japanese higher via Kana detection
        let japanese = "こんにちは世界"; // Has 3 Hiragana + 2 Chinese chars
        let result = detect_language(japanese);
        // Should be Japanese due to Hiragana presence
        assert_eq!(result.language, Language::Japanese);
    }

    #[test]
    fn test_minimal_cjk_threshold() {
        // Very low CJK content should still detect the language
        let result = detect_language("hello 世界");
        assert!(result.ratio > 0.0);
        assert!(result.ratio < 1.0);
    }
}
