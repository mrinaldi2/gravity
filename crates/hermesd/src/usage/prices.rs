//! API-equivalent prices, turning tokens into cost units (USD) that compare
//! across bots, models and providers. Subscription usage is not billed this
//! way; the unit is only a common scale for shares (H-023 §2.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::chat::usage_lines::Tokens;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    /// Cache writes with the 5-minute lifetime.
    pub cache_write: f64,
    /// Cache writes with the 1-hour lifetime.
    pub cache_write_1h: f64,
    pub cache_read: f64,
    pub output: f64,
}

impl ModelPrice {
    /// The usual shape: cache writes at 1.25× (5m) and 2× (1h) input.
    const fn standard(input: f64, cache_read: f64, output: f64) -> Self {
        Self {
            input,
            cache_write: input * 1.25,
            cache_write_1h: input * 2.0,
            cache_read,
            output,
        }
    }

    /// Cost units for `tokens`. Reasoning tokens are already in `output`.
    pub fn units(&self, tokens: &Tokens) -> f64 {
        (tokens.input as f64 * self.input
            + tokens.cache_write as f64 * self.cache_write
            + tokens.cache_write_1h as f64 * self.cache_write_1h
            + tokens.cache_read as f64 * self.cache_read
            + tokens.output as f64 * self.output)
            / 1_000_000.0
    }
}

/// Built-in prices by model-id prefix (Anthropic first-party rates,
/// 2026-09). The longest matching prefix wins, so a family entry covers
/// dated and suffixed ids without shadowing a newer point release.
const BUILT_IN: &[(&str, ModelPrice)] = &[
    ("claude-fable-5-1", ModelPrice::standard(10.0, 0.25, 50.0)),
    ("claude-mythos-5-1", ModelPrice::standard(10.0, 0.25, 50.0)),
    ("claude-fable-5", ModelPrice::standard(10.0, 1.0, 50.0)),
    ("claude-mythos-5", ModelPrice::standard(10.0, 1.0, 50.0)),
    ("claude-opus-5-5", ModelPrice::standard(4.0, 0.20, 20.0)),
    ("claude-opus-5", ModelPrice::standard(5.0, 0.50, 25.0)),
    ("claude-opus-4", ModelPrice::standard(5.0, 0.50, 25.0)),
    // Opus 4 and 4.1 kept the old Opus price.
    ("claude-opus-4-1", ModelPrice::standard(15.0, 1.50, 75.0)),
    ("claude-opus-4-2025", ModelPrice::standard(15.0, 1.50, 75.0)),
    ("claude-sonnet-5", ModelPrice::standard(2.0, 0.20, 10.0)),
    ("claude-sonnet-4", ModelPrice::standard(3.0, 0.30, 15.0)),
    ("claude-haiku-4", ModelPrice::standard(1.0, 0.10, 5.0)),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    /// Count bots' token usage into `usage_minute`.
    pub enabled: bool,
    /// How often transcripts are scanned for new usage.
    pub interval_seconds: u64,
    /// Prices by model-id prefix, USD per million tokens. Entries here
    /// override the built-in table and add models it lacks, e.g.
    /// `[usage.prices."claude-opus-5-5"]` with `input`, `cache_write`,
    /// `cache_write_1h`, `cache_read` and `output`.
    pub prices: BTreeMap<String, ModelPrice>,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_seconds: 30,
            prices: BTreeMap::new(),
        }
    }
}

impl UsageConfig {
    /// The price for `model`, or `None` for a model nobody priced: its
    /// tokens are still counted, at zero units.
    pub fn price(&self, model: &str) -> Option<ModelPrice> {
        let model = normalize(model);
        let configured = self
            .prices
            .iter()
            .filter(|(prefix, _)| model.starts_with(&normalize(prefix)))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(_, p)| *p);
        configured.or_else(|| {
            BUILT_IN
                .iter()
                .filter(|(prefix, _)| model.starts_with(prefix))
                .max_by_key(|(prefix, _)| prefix.len())
                .map(|(_, p)| *p)
        })
    }
}

/// Lower-cased, without a context-window tag such as `[1m]`.
fn normalize(model: &str) -> String {
    model
        .split('[')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        let cfg = UsageConfig::default();
        assert_eq!(cfg.price("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(cfg.price("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(cfg.price("claude-opus-4-1-20250805").unwrap().input, 15.0);
        assert_eq!(cfg.price("claude-opus-4-8").unwrap().input, 5.0);
        assert_eq!(cfg.price("claude-fable-5-1[1m]").unwrap().cache_read, 0.25);
        assert_eq!(cfg.price("claude-haiku-4-5-20251001").unwrap().output, 5.0);
        assert!(cfg.price("gpt-5").is_none());
    }

    #[test]
    fn configured_prices_override_and_extend() {
        let cfg: UsageConfig = toml::from_str(
            r#"
            [prices."claude-opus-5-5"]
            input = 1.0
            cache_write = 1.0
            cache_write_1h = 1.0
            cache_read = 1.0
            output = 1.0
            [prices."local-model"]
            input = 0.5
            cache_write = 0.0
            cache_write_1h = 0.0
            cache_read = 0.0
            output = 0.5
            "#,
        )
        .unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.price("claude-opus-5-5").unwrap().output, 1.0);
        assert_eq!(cfg.price("local-model-v2").unwrap().input, 0.5);
        // Untouched models keep the built-in price.
        assert_eq!(cfg.price("claude-sonnet-5-5").unwrap().input, 2.0);
    }

    #[test]
    fn units_price_each_kind_of_token() {
        let price = ModelPrice::standard(4.0, 0.20, 20.0);
        let tokens = Tokens {
            input: 1_000_000,
            cache_write: 1_000_000,
            cache_write_1h: 1_000_000,
            cache_read: 1_000_000,
            output: 1_000_000,
            reasoning: 500_000,
        };
        // 4 + 5 + 8 + 0.2 + 20; reasoning is inside output.
        assert!((price.units(&tokens) - 37.2).abs() < 1e-9);
    }
}
