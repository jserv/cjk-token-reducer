use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const STATS_FILENAME: &str = "stats.json";
const MAX_SESSIONS: usize = 30;

// Claude pricing per million tokens (as of 2024)
const INPUT_COST_PER_MTOK: f64 = 15.0;
const OUTPUT_COST_PER_MTOK: f64 = 75.0;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStats {
    pub total_translations: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub estimated_saved_tokens: u64,
    pub sessions: Vec<SessionStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionStats {
    pub date: NaiveDate,
    pub translations: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_saved: u64,
}

fn stats_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cjk-token-reducer")
        .join(STATS_FILENAME)
}

/// Load stats from disk or return empty stats
pub fn load_stats() -> TokenStats {
    load_stats_from_path(&stats_path())
}

/// Load stats from a specific path (for testing)
pub fn load_stats_from_path(path: &std::path::Path) -> TokenStats {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save stats to a specific path using atomic write (temp file + rename)
///
/// This ensures that if the process crashes during write, the original
/// stats file remains intact. The rename operation is atomic on most
/// filesystems (POSIX guarantees this for same-filesystem renames).
pub fn save_stats_to_path(path: &std::path::Path, stats: &TokenStats) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let json = match serde_json::to_string_pretty(stats) {
        Ok(j) => j,
        Err(_) => return,
    };

    // Create a temp file in the same directory (ensures same filesystem for atomic rename)
    let temp_path = path.with_extension("json.tmp");

    // Write to temp file first
    if std::fs::write(&temp_path, &json).is_err() {
        return;
    }

    // Atomic rename: if this fails, the original file is untouched
    let _ = std::fs::rename(&temp_path, path);
}

/// Record a translation event
pub fn record_translation(input_tokens: usize, output_tokens: usize) {
    record_translation_to_path(&stats_path(), input_tokens, output_tokens);
}

/// Record a translation event to a specific path (for testing)
pub fn record_translation_to_path(
    path: &std::path::Path,
    input_tokens: usize,
    output_tokens: usize,
) {
    let mut stats = load_stats_from_path(path);
    let today = Utc::now().date_naive();

    // Estimate tokens saved: input_tokens is the estimated CJK tokens,
    // output_tokens is the estimated English tokens after translation.
    let estimated_saved = (input_tokens as u64).saturating_sub(output_tokens as u64);

    stats.total_translations += 1;
    stats.total_input_tokens += input_tokens as u64;
    stats.total_output_tokens += output_tokens as u64;
    stats.estimated_saved_tokens += estimated_saved;

    // Find or create today's session
    if let Some(session) = stats.sessions.iter_mut().find(|s| s.date == today) {
        session.translations += 1;
        session.input_tokens += input_tokens as u64;
        session.output_tokens += output_tokens as u64;
        session.estimated_saved += estimated_saved;
    } else {
        stats.sessions.push(SessionStats {
            date: today,
            translations: 1,
            input_tokens: input_tokens as u64,
            output_tokens: output_tokens as u64,
            estimated_saved,
        });
    }

    // Keep only last 30 sessions
    if stats.sessions.len() > MAX_SESSIONS {
        stats.sessions = stats
            .sessions
            .split_off(stats.sessions.len() - MAX_SESSIONS);
    }

    save_stats_to_path(path, &stats);
}

/// Estimate cost savings based on Claude pricing (assumes 50/50 input/output split)
fn estimate_cost_savings(saved_tokens: u64) -> f64 {
    let avg_cost_per_mtok = (INPUT_COST_PER_MTOK + OUTPUT_COST_PER_MTOK) / 2.0;
    (saved_tokens as f64 * avg_cost_per_mtok) / 1_000_000.0
}

/// Format stats for display
pub fn format_stats(stats: &TokenStats) -> String {
    let cost_saved = estimate_cost_savings(stats.estimated_saved_tokens);

    format!(
        r#"
╔══════════════════════════════════════════════════════════╗
║           Claude CJK Token Statistics                    ║
╠══════════════════════════════════════════════════════════╣
║  Total Translations:     {:>10}                      ║
║  Translation Tokens:     {:>10}                      ║
║  Estimated Saved:        {:>10}                      ║
║  Est. Cost Saved:        ${:>9.4}                      ║
╚══════════════════════════════════════════════════════════╝
"#,
        stats.total_translations,
        stats.total_input_tokens + stats.total_output_tokens,
        stats.estimated_saved_tokens,
        cost_saved
    )
}

/// Export stats as JSON
pub fn format_stats_json(stats: &TokenStats) -> String {
    serde_json::to_string_pretty(stats).unwrap_or_else(|_| "{}".to_string())
}

/// Export stats as CSV
pub fn format_stats_csv(stats: &TokenStats) -> String {
    let mut lines =
        vec!["date,translations,input_tokens,output_tokens,estimated_saved".to_string()];
    for session in &stats.sessions {
        lines.push(format!(
            "{},{},{},{},{}",
            session.date,
            session.translations,
            session.input_tokens,
            session.output_tokens,
            session.estimated_saved
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_stats() {
        let stats = TokenStats::default();
        assert_eq!(stats.total_translations, 0);
        assert!(stats.sessions.is_empty());
    }

    #[test]
    fn test_format_empty_stats() {
        let stats = TokenStats::default();
        let output = format_stats(&stats);
        assert!(output.contains("Total Translations:"));
        assert!(output.contains("0")); // Zero translations
    }

    #[test]
    fn test_estimate_cost_savings() {
        let saved_tokens = 1_000_000; // 1M tokens saved
        let cost = estimate_cost_savings(saved_tokens);

        // With the formula: (saved_tokens as f64 * avg_cost_per_mtok) / 1_000_000.0
        // avg_cost_per_mtok = (15.0 + 75.0) / 2.0 = 45.0
        // So cost should be (1_000_000 * 45.0) / 1_000_000.0 = 45.0
        assert_eq!(cost, 45.0);
    }

    #[test]
    fn test_record_translation_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_path = temp_dir.path().join("test_stats.json");

        // Record stats using the path-based function
        record_translation_to_path(&test_path, 100, 80);

        // Verify
        let loaded = load_stats_from_path(&test_path);
        assert_eq!(loaded.total_translations, 1);
        assert_eq!(loaded.total_input_tokens, 100);
        assert_eq!(loaded.total_output_tokens, 80);
        assert_eq!(loaded.estimated_saved_tokens, 20);
    }

    #[test]
    fn test_format_stats_json() {
        let stats = TokenStats {
            total_translations: 5,
            total_input_tokens: 1000,
            total_output_tokens: 800,
            estimated_saved_tokens: 200,
            ..Default::default()
        };

        let json_output = format_stats_json(&stats);
        assert!(json_output.contains("totalTranslations"));
        assert!(json_output.contains("5"));
        assert!(json_output.contains("1000"));
    }

    #[test]
    fn test_format_stats_csv() {
        let mut stats = TokenStats::default();
        let today = Utc::now().date_naive();
        stats.sessions.push(SessionStats {
            date: today,
            translations: 2,
            input_tokens: 200,
            output_tokens: 150,
            estimated_saved: 50,
        });

        let csv_output = format_stats_csv(&stats);
        assert!(
            csv_output.starts_with("date,translations,input_tokens,output_tokens,estimated_saved")
        );
        assert!(csv_output.contains(&today.to_string()));
        assert!(csv_output.contains("2,200,150,50"));
    }

    #[test]
    fn test_session_limit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let test_path = temp_dir.path().join("test_stats_limit.json");

        // Create more than MAX_SESSIONS sessions
        let mut stats = TokenStats::default();
        for i in 0..MAX_SESSIONS + 5 {
            let past_date = Utc::now().date_naive() - chrono::Duration::days(i as i64);
            stats.sessions.push(SessionStats {
                date: past_date,
                translations: 1,
                input_tokens: 100,
                output_tokens: 80,
                estimated_saved: 20,
            });

            // Keep only the last MAX_SESSIONS
            if stats.sessions.len() > MAX_SESSIONS {
                stats
                    .sessions
                    .drain(0..(stats.sessions.len() - MAX_SESSIONS));
            }
        }

        save_stats_to_path(&test_path, &stats);
        let loaded = load_stats_from_path(&test_path);
        assert!(loaded.sessions.len() <= MAX_SESSIONS);
    }

    #[test]
    fn test_stats_path() {
        let path = stats_path();
        assert!(path.ends_with("cjk-token-reducer/stats.json"));
    }

    #[test]
    fn test_load_stats_default() {
        // Test loading default stats from a non-existent file
        let temp_dir = tempfile::tempdir().unwrap();
        let nonexistent_path = temp_dir.path().join("nonexistent_stats.json");

        let stats = load_stats_from_path(&nonexistent_path);
        assert_eq!(stats.total_translations, 0);
        assert_eq!(stats.total_input_tokens, 0);
        assert_eq!(stats.total_output_tokens, 0);
        assert_eq!(stats.estimated_saved_tokens, 0);
        assert!(stats.sessions.is_empty());
    }

    #[test]
    fn test_save_stats() {
        // Use temp directory to avoid modifying global stats
        let temp_dir = tempfile::tempdir().unwrap();
        let test_path = temp_dir.path().join("test_save_stats.json");

        let stats = TokenStats {
            total_translations: 5,
            total_input_tokens: 1000,
            total_output_tokens: 800,
            estimated_saved_tokens: 200,
            ..Default::default()
        };

        // Save using public function
        save_stats_to_path(&test_path, &stats);

        // Verify the file was created and contains the data
        assert!(test_path.exists());

        let loaded_stats = load_stats_from_path(&test_path);
        assert_eq!(loaded_stats.total_translations, 5);
        assert_eq!(loaded_stats.total_input_tokens, 1000);
        assert_eq!(loaded_stats.total_output_tokens, 800);
        assert_eq!(loaded_stats.estimated_saved_tokens, 200);
    }

    #[test]
    fn test_record_translation() {
        // Use temp directory for isolated testing
        let temp_dir = tempfile::tempdir().unwrap();
        let test_path = temp_dir.path().join("test_record.json");

        // Record first translation
        record_translation_to_path(&test_path, 100, 80);

        let stats = load_stats_from_path(&test_path);
        assert_eq!(stats.total_translations, 1);
        assert_eq!(stats.total_input_tokens, 100);
        assert_eq!(stats.total_output_tokens, 80);
        assert_eq!(stats.estimated_saved_tokens, 20);
        assert_eq!(stats.sessions.len(), 1);

        // Record second translation
        record_translation_to_path(&test_path, 200, 150);

        let stats = load_stats_from_path(&test_path);
        assert_eq!(stats.total_translations, 2);
        assert_eq!(stats.total_input_tokens, 300);
        assert_eq!(stats.total_output_tokens, 230);
        assert_eq!(stats.estimated_saved_tokens, 70);
        // Same day, so still one session
        assert_eq!(stats.sessions.len(), 1);
    }

    #[test]
    fn test_format_stats() {
        let stats = TokenStats {
            total_translations: 10,
            total_input_tokens: 1000,
            total_output_tokens: 800,
            estimated_saved_tokens: 200,
            ..Default::default()
        };

        let output = format_stats(&stats);
        assert!(output.contains("Total Translations:"));
        assert!(output.contains("1800")); // input + output tokens
        assert!(output.contains("200")); // estimated saved
    }

    #[test]
    fn test_avg_cost_per_mtok_calculation() {
        // Verify the average cost calculation
        let avg_cost = (INPUT_COST_PER_MTOK + OUTPUT_COST_PER_MTOK) / 2.0;
        assert_eq!(avg_cost, 45.0);
    }
}
