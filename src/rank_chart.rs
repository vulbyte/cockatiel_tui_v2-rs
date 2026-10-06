//! The rank chart — the repo-root, streamer-editable mapping of 0-1 rank to a
//! display tier name, shared with the engine and term-chat.
//!
//! Numbers are for logic (the 0-1 rank on the wire); names are for display.
//! Tier order in the file is arbitrary; consumers sort by `min` ascending and
//! a rank maps to the highest tier whose `min <= rank`.

use serde::Deserialize;
use std::sync::OnceLock;

/// One tier entry: a display name + the 0-1 rank it starts at.
#[derive(Debug, Clone, Deserialize)]
pub struct RankTier {
    pub name: String,
    pub min: f32,
}

/// The parsed + sorted chart.
#[derive(Debug, Clone, Default)]
pub struct RankChart {
    tiers: Vec<RankTier>,
}

impl RankChart {
    /// Parse + sort ascending by `min`.
    pub fn from_json(data: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Chart {
            #[serde(default)]
            tiers: Vec<RankTier>,
        }
        let chart: Chart =
            serde_json::from_str(data).map_err(|e| format!("rank_chart.json parse error: {e}"))?;
        let mut tiers = chart.tiers;
        tiers.sort_by(|a, b| a.min.total_cmp(&b.min));
        Ok(Self { tiers })
    }

    /// The display name for a 0-1 rank (highest tier with `min <= rank`).
    pub fn name_for_rank(&self, rank: f32) -> &str {
        let mut name = "—";
        for t in &self.tiers {
            if rank >= t.min {
                name = &t.name;
            } else {
                break;
            }
        }
        name
    }
}

fn default_chart_path() -> std::path::PathBuf {
    // The environment variable the supervisor injects into children always
    // wins; otherwise the runtime-resolved chart (installed `<root>/rank_chart.json`,
    // legacy repo root) applies.
    if let Ok(p) = std::env::var("COCKATIEL_RANK_CHART") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    crate::paths::current().rank_chart.clone()
}

static CHART: OnceLock<RankChart> = OnceLock::new();

/// The loaded chart (cached). Missing/unparseable → the default mineral
/// template so display never breaks.
pub fn chart() -> &'static RankChart {
    CHART.get_or_init(|| {
        std::fs::read_to_string(default_chart_path())
            .ok()
            .and_then(|s| RankChart::from_json(&s).ok())
            .unwrap_or_else(default_chart)
    })
}

/// The built-in mineral template.
pub fn default_chart() -> RankChart {
    RankChart::from_json(
        r#"{
  "tiers": [
    { "name": "coal",     "min": 0.0 },
    { "name": "copper",   "min": 0.1 },
    { "name": "bronze",   "min": 0.2 },
    { "name": "silver",   "min": 0.3 },
    { "name": "gold",     "min": 0.4 },
    { "name": "sapphire", "min": 0.5 },
    { "name": "emerald",  "min": 0.6 },
    { "name": "ruby",     "min": 0.7 },
    { "name": "diamond",  "min": 0.8 },
    { "name": "opal",     "min": 0.9 }
  ]
}"#,
    )
    .expect("built-in mineral template must parse")
}

/// The tier name for a rank.
pub fn tier_name(rank: f32) -> String {
    chart().name_for_rank(rank).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_sorts_and_looks_up_highest_min() {
        let c = RankChart::from_json(
            r#"{"tiers":[
                {"name":"opal","min":0.9},
                {"name":"coal","min":0.0},
                {"name":"silver","min":0.3},
                {"name":"gold","min":0.4}
            ]}"#,
        )
        .unwrap();
        assert_eq!(c.name_for_rank(0.0), "coal");
        assert_eq!(c.name_for_rank(0.29), "coal");
        assert_eq!(c.name_for_rank(0.3), "silver");
        assert_eq!(c.name_for_rank(0.39), "silver");
        assert_eq!(c.name_for_rank(0.4), "gold");
        assert_eq!(c.name_for_rank(0.9), "opal");
        assert_eq!(c.name_for_rank(1.0), "opal");
    }

    #[test]
    fn default_template_is_full_mineral_ladder() {
        let c = default_chart();
        let names: Vec<String> = c.tiers.iter().map(|t| t.name.clone()).collect();
        assert_eq!(
            names,
            vec![
                "coal", "copper", "bronze", "silver", "gold", "sapphire",
                "emerald", "ruby", "diamond", "opal"
            ]
        );
    }
}