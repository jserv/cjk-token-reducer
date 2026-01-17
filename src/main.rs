use cjk_token_reducer::{
    cache::{format_cache_stats, TranslationCache},
    config::load_config,
    detector::{detect_language, Language},
    output::{print_error, print_sensitive_warning, print_verbose, Colorize},
    preserver::{extract_and_preserve_with_config, SegmentType},
    security::sanitize_for_log,
    stats::{format_stats, format_stats_csv, format_stats_json, load_stats, record_translation},
    tokenizer::{count_tokens_with_fallback, tokenize_with_fallback},
    translator::{build_output_language_instruction, translate_to_english_with_options},
};
use serde::{Deserialize, Serialize};
use std::io::{self, IsTerminal, Read};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct HookInput {
    prompt: String,
}

#[derive(Serialize)]
struct HookOutput {
    prompt: String,
}

/// Read prompt from stdin, supporting both JSON and plain text formats
///
/// If stdin is a terminal (no piped input), returns None with an error message.
fn read_prompt_from_stdin() -> Option<String> {
    // Check if stdin is a terminal (no piped input)
    if io::stdin().is_terminal() {
        print_error("No input provided. Pipe text to this command:");
        eprintln!("  echo 'your text' | cjk-token-reducer --tokenize");
        eprintln!("  echo '{{\"prompt\": \"your text\"}}' | cjk-token-reducer");
        return None;
    }

    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        print_error("Failed to read stdin");
        return None;
    }

    if input.trim().is_empty() {
        return Some(String::new());
    }

    // Try JSON parse, fallback to plain text
    // Always trim to ensure consistency between JSON and plain text input
    Some(match serde_json::from_str::<HookInput>(&input) {
        Ok(hook) => hook.prompt.trim().to_string(),
        Err(_) => input.trim().to_string(),
    })
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_cache = !args.iter().any(|s| s == "--no-cache");
    let verbose = args.iter().any(|s| s == "--verbose" || s == "-v");

    // Handle CLI commands
    match args.get(1).map(String::as_str) {
        Some("--stats") => {
            let stats = load_stats();
            // Check for export format
            if args.iter().any(|s| s == "--json") {
                println!("{}", format_stats_json(&stats));
            } else if args.iter().any(|s| s == "--csv") {
                println!("{}", format_stats_csv(&stats));
            } else {
                println!("{}", format_stats(&stats));
            }
            return;
        }
        Some("--cache-stats") => {
            handle_cache_stats();
            return;
        }
        Some("--clear-cache") => {
            handle_clear_cache();
            return;
        }
        Some("--version" | "-V") => {
            println!("cjk-token-reducer {VERSION}");
            return;
        }
        Some("--help" | "-h") => {
            print_help();
            return;
        }
        Some("--dry-run") => {
            handle_dry_run();
            return;
        }
        Some("--tokenize") => {
            handle_tokenize(&args);
            return;
        }
        Some("--show-preserved") => {
            handle_show_preserved();
            return;
        }
        _ => {}
    }

    print_verbose(&format!("Cache enabled: {use_cache}"), verbose);

    let prompt = match read_prompt_from_stdin() {
        Some(p) if p.is_empty() => {
            let output = HookOutput {
                prompt: String::new(),
            };
            println!("{}", serde_json::to_string(&output).unwrap());
            return;
        }
        Some(p) => p,
        None => std::process::exit(1),
    };

    let config = load_config();

    print_verbose(&format!("Input length: {} chars", prompt.len()), verbose);

    match translate_to_english_with_options(&prompt, &config, use_cache).await {
        Ok(result) => {
            print_verbose(
                &format!(
                    "Language: {:?}, translated: {}, cache_hit: {}",
                    result.source_language, result.was_translated, result.cache_hit
                ),
                verbose,
            );

            let mut output_text = result.translated.clone();

            // Add output language instruction if needed
            if result.was_translated && config.output_language != "en" {
                output_text.push_str(&build_output_language_instruction(&config.output_language));
            }

            // Record stats if enabled
            if result.was_translated && config.enable_stats {
                record_translation(result.input_tokens, result.output_tokens);
                print_verbose(
                    &format!(
                        "Tokens: {} → {} (saved ~{})",
                        result.input_tokens,
                        result.output_tokens,
                        result.input_tokens.saturating_sub(result.output_tokens)
                    ),
                    verbose,
                );
            }

            // Output JSON
            let output = HookOutput {
                prompt: output_text,
            };
            println!("{}", serde_json::to_string(&output).unwrap());
        }
        Err(e) => {
            print_error(&format!("Translation failed: {e}"));
            // Fallback: return original
            let output = HookOutput { prompt };
            println!("{}", serde_json::to_string(&output).unwrap());
        }
    }
}

fn handle_cache_stats() {
    let config = load_config();
    match TranslationCache::open(&config.cache) {
        Ok(cache) => println!("{}", format_cache_stats(&cache.stats())),
        Err(e) => {
            print_error(&format!("Failed to open cache: {e}"));
            std::process::exit(1);
        }
    }
}

fn handle_clear_cache() {
    let config = load_config();
    match TranslationCache::open(&config.cache) {
        Ok(cache) => match cache.clear() {
            Ok(_) => println!("{}", "[cjk-token] Cache cleared successfully".green()),
            Err(e) => {
                print_error(&format!("Failed to clear cache: {e}"));
                std::process::exit(1);
            }
        },
        Err(e) => {
            print_error(&format!("Failed to open cache: {e}"));
            std::process::exit(1);
        }
    }
}

fn handle_dry_run() {
    let prompt = match read_prompt_from_stdin() {
        Some(p) if p.is_empty() => {
            print_error("No input provided");
            std::process::exit(1);
        }
        Some(p) => p,
        None => std::process::exit(1),
    };

    // Security: warn about sensitive data in debug output
    print_sensitive_warning();

    let config = load_config();
    let detection = detect_language(&prompt);
    let preserve_config = (&config.preserve).into();
    let preserved = extract_and_preserve_with_config(&prompt, &preserve_config);

    println!("{}", "Dry Run Analysis".bold().underline());
    println!();
    println!("{}: {:?}", "Detected Language".cyan(), detection.language);
    println!("{}: {:.1}%", "CJK Ratio".cyan(), detection.ratio * 100.0);
    println!(
        "{}: {} (threshold: {})",
        "Would Translate".cyan(),
        if detection.ratio >= config.threshold && detection.language != Language::English {
            "Yes".green()
        } else {
            "No".yellow()
        },
        config.threshold
    );
    println!(
        "{}: {}",
        "Preserved Segments".cyan(),
        preserved.segments.len()
    );

    if !preserved.segments.is_empty() {
        for seg in &preserved.segments {
            let truncated = if seg.original.len() > 50 {
                format!("{}...", &seg.original[..47])
            } else {
                seg.original.clone()
            };
            println!("  {:?}: {}", seg.segment_type, truncated.dimmed());
        }
    }

    println!();
    println!("{}: {} chars", "Input Length".cyan(), prompt.len());
    println!(
        "{}: ~{} tokens",
        "Estimated Input Tokens".cyan(),
        (prompt.chars().count() as f64 * 2.0).ceil() as usize
    );
}

fn handle_show_preserved() {
    let prompt = match read_prompt_from_stdin() {
        Some(p) if p.is_empty() => {
            print_error("No input provided");
            std::process::exit(1);
        }
        Some(p) => p,
        None => std::process::exit(1),
    };

    // Security: warn about sensitive data in debug output
    print_sensitive_warning();

    let config = load_config();
    let preserve_config = (&config.preserve).into();
    let preserved = extract_and_preserve_with_config(&prompt, &preserve_config);

    println!("{}", "Preserved Segments Analysis".bold().underline());
    println!();

    // Helper to filter segments by type
    let filter_by_type = |seg_type: SegmentType| -> Vec<_> {
        preserved
            .segments
            .iter()
            .filter(|s| s.segment_type == seg_type)
            .collect()
    };

    let code_blocks = filter_by_type(SegmentType::CodeBlock);
    let inline_code = filter_by_type(SegmentType::InlineCode);
    let urls = filter_by_type(SegmentType::Url);
    let paths = filter_by_type(SegmentType::FilePath);
    let no_translate = filter_by_type(SegmentType::NoTranslate);
    let english_terms = filter_by_type(SegmentType::EnglishTerm);

    // Print summary
    println!(
        "{}: {}",
        "Total Preserved".cyan().bold(),
        preserved.segments.len()
    );
    println!();

    // Print each category
    if !code_blocks.is_empty() {
        println!("{} ({})", "Code Blocks".green().bold(), code_blocks.len());
        for seg in &code_blocks {
            let preview = if seg.original.len() > 60 {
                format!("{}...", &seg.original[..57])
            } else {
                seg.original.clone()
            };
            println!("  {}", preview.replace('\n', "\\n").dimmed());
        }
        println!();
    }

    if !inline_code.is_empty() {
        println!("{} ({})", "Inline Code".green().bold(), inline_code.len());
        for seg in &inline_code {
            println!("  {}", seg.original.dimmed());
        }
        println!();
    }

    if !no_translate.is_empty() {
        println!(
            "{} ({})",
            "No-Translate Markers".yellow().bold(),
            no_translate.len()
        );
        for seg in &no_translate {
            println!("  {} (markers stripped)", seg.original.yellow());
        }
        println!();
    }

    if !english_terms.is_empty() {
        println!(
            "{} ({})",
            "English Technical Terms".blue().bold(),
            english_terms.len()
        );
        for seg in &english_terms {
            println!("  {}", seg.original.blue());
        }
        println!();
    }

    if !urls.is_empty() {
        println!("{} ({})", "URLs".cyan().bold(), urls.len());
        for seg in &urls {
            println!("  {}", seg.original.dimmed());
        }
        println!();
    }

    if !paths.is_empty() {
        println!("{} ({})", "File Paths".cyan().bold(), paths.len());
        for seg in &paths {
            println!("  {}", seg.original.dimmed());
        }
        println!();
    }

    // Show text with placeholders
    println!("{}", "Text with Placeholders".bold());
    println!("{}", preserved.text.dimmed());
}

fn handle_tokenize(args: &[String]) {
    let prompt = match read_prompt_from_stdin() {
        Some(p) if p.is_empty() => {
            print_error("No input provided");
            std::process::exit(1);
        }
        Some(p) => p,
        None => std::process::exit(1),
    };

    let show_tokens = args.iter().any(|s| s == "--show-tokens");
    let json_output = args.iter().any(|s| s == "--json");
    let include_text = args.iter().any(|s| s == "--include-text");
    let detection = detect_language(&prompt);

    // Security: warn about sensitive data in debug output (unless JSON-only)
    if !json_output {
        print_sensitive_warning();
    }

    // Use fallback-aware API
    let token_result = count_tokens_with_fallback(&prompt);
    let token_count = token_result.count;
    let (tokens, tokenize_fallback) = if show_tokens {
        tokenize_with_fallback(&prompt)
    } else {
        (vec![], false)
    };
    let used_fallback = token_result.used_fallback || tokenize_fallback;

    if json_output {
        // Security: only include full text if explicitly requested with --include-text
        // This prevents accidental exposure of prompt contents in logs
        let text_field: Option<&str> = if include_text { Some(&prompt) } else { None };
        let text_preview = sanitize_for_log(&prompt, 50);

        let output = serde_json::json!({
            "text": text_field,
            "text_preview": text_preview,
            "language": format!("{:?}", detection.language),
            "cjk_ratio": detection.ratio,
            "token_count": token_count,
            "tokens": if show_tokens { Some(&tokens) } else { None },
            "char_count": prompt.chars().count(),
            "byte_count": prompt.len(),
            "used_fallback": used_fallback,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
        return;
    }

    // Claude pricing (per million tokens) - Opus pricing as reference
    const INPUT_COST_PER_MTOK: f64 = 15.0;
    let estimated_cost = (token_count as f64 * INPUT_COST_PER_MTOK) / 1_000_000.0;

    println!("{}", "Token Analysis".bold().underline());
    if used_fallback {
        println!("{}", "(using fallback estimation)".yellow());
    }
    println!();
    println!("{}: {:?}", "Detected Language".cyan(), detection.language);
    println!("{}: {:.1}%", "CJK Ratio".cyan(), detection.ratio * 100.0);
    println!();
    let count_label = if used_fallback {
        "Token Count (est)".cyan().bold()
    } else {
        "Token Count".cyan().bold()
    };
    println!(
        "{}: {}",
        count_label,
        token_count.to_string().green().bold()
    );
    println!("{}: {}", "Character Count".cyan(), prompt.chars().count());
    println!("{}: {}", "Byte Count".cyan(), prompt.len());
    println!(
        "{}: ${:.6} {}",
        "Est. Input Cost".cyan(),
        estimated_cost,
        "(Opus)".dimmed()
    );

    if show_tokens {
        println!();
        if tokens.is_empty() {
            println!("{}: {}", "Tokens".cyan().bold(), "(tokenizer failed)".red());
        } else {
            println!("{}", "Tokens".cyan().bold());
            for (i, token) in tokens.iter().enumerate() {
                let display = token.replace('\n', "\\n").replace('\t', "\\t");
                if display.trim().is_empty() {
                    println!("  {:>4}: {:?}", i + 1, display.dimmed());
                } else {
                    println!("  {:>4}: {}", i + 1, display);
                }
            }
        }
    }

    // If CJK content, show potential savings estimate
    if detection.ratio > 0.1 && detection.language != Language::English {
        println!();
        println!("{}", "Savings Estimate".cyan().bold());
        // Weight reduction factor by CJK ratio:
        // 100% CJK -> 40% reduction (factor 0.6)
        // Mixed content -> proportionally less reduction
        let reduction_factor = 1.0 - (0.4 * detection.ratio);
        let estimated_english_tokens = (token_count as f64 * reduction_factor).ceil() as usize;
        let potential_saved = token_count.saturating_sub(estimated_english_tokens);
        let savings_pct = if token_count > 0 {
            (potential_saved as f64 / token_count as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  {} → {} tokens ({:.0}% reduction)",
            token_count.to_string().yellow(),
            estimated_english_tokens.to_string().green(),
            savings_pct
        );
        println!(
            "  Potential savings: {} tokens (${:.6})",
            potential_saved.to_string().green(),
            (potential_saved as f64 * INPUT_COST_PER_MTOK) / 1_000_000.0
        );
    }
}

fn print_help() {
    println!(
        r#"
CJK Token Reducer - Reduce token usage by translating CJK to English

Usage:
  As Claude Code Hook:
    Add to your Claude Code hooks configuration

  CLI Commands:
    cjk-token-reducer --stats        Show token savings statistics
    cjk-token-reducer --stats --json Export stats as JSON
    cjk-token-reducer --stats --csv  Export stats as CSV
    cjk-token-reducer --tokenize     Show precise token count (Claude tokenizer)
    cjk-token-reducer --tokenize --show-tokens  Show individual tokens
    cjk-token-reducer --tokenize --json         Export token analysis as JSON
    cjk-token-reducer --tokenize --json --include-text  Include full text in JSON
    cjk-token-reducer --cache-stats  Show translation cache statistics
    cjk-token-reducer --clear-cache  Clear the translation cache
    cjk-token-reducer --dry-run      Preview detection without translation
    cjk-token-reducer --show-preserved  Show detailed preserved segments analysis
    cjk-token-reducer --no-cache     Bypass cache for this translation
    cjk-token-reducer --verbose, -v  Show detailed processing info
    cjk-token-reducer --version, -V  Show version number
    cjk-token-reducer --help, -h     Show this help message

Environment Variables:
    CJK_TOKEN_OUTPUT_LANG    Override output language (en, zh, ja, ko)
    CJK_TOKEN_THRESHOLD      Override CJK detection threshold (0.0-1.0)
    CJK_TOKEN_CACHE_ENABLED  Override cache enabled (true/false)

Supported Languages:
  - Chinese (中文)
  - Japanese (日本語)
  - Korean (한국어)

No-Translate Markers:
  Use [[text]] or ==text== to prevent specific text from being translated:
    Input:  이 함수는 [[getUserData]]를 호출합니다
    Output: This function calls getUserData

Security:
  - Debug commands (--dry-run, --show-preserved, --tokenize) display warnings
    about potential sensitive data exposure in output
  - JSON output from --tokenize excludes full text by default (use --include-text)
  - API keys and prompt contents are never written to log files

Configuration:
  Create a .cjk-token.json file in your project or home directory:

  {{
    "outputLanguage": "en",
    "threshold": 0.1,
    "enableStats": true,
    "cache": {{
      "enabled": true,
      "ttlDays": 30,
      "maxSizeMb": 10
    }},
    "preserve": {{
      "wikiMarkers": true,
      "highlightMarkers": true,
      "englishTerms": true
    }}
  }}
"#
    );
}
