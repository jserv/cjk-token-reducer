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

fn ensure_stats_dir() {
    if let Some(parent) = stats_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

/// Load stats from disk or return empty stats
pub fn load_stats() -> TokenStats {
    ensure_stats_dir();
    std::fs::read_to_string(stats_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_stats(stats: &TokenStats) {
    ensure_stats_dir();
    if let Ok(json) = serde_json::to_string_pretty(stats) {
        let _ = std::fs::write(stats_path(), json);
    }
}

/// Record a translation event
pub fn record_translation(input_tokens: usize, output_tokens: usize) {
    let mut stats = load_stats();
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

    save_stats(&stats);
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
}
