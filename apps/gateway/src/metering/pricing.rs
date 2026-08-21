//! Model pricing.
//!
//! Every cost, baseline, and savings figure in Aegis resolves through this table, which
//! makes it the single most correctness-critical data structure in the product: a wrong
//! number here produces a wrong invoice, and Part 13 item 1 says a penny-wrong invoice
//! destroys trust.
//!
//! # Provenance
//!
//! Part 13 item 8 requires every price to be traceable to a dated source. Each seed entry
//! carries [`ModelPricing::source`], a `provider — as-of date` string. The seed data is a
//! **development bootstrap**: before billing a real customer, run the verification
//! procedure in `docs/runbooks/pricing-update.md`, which re-checks every row against the
//! provider's published price sheet and writes the confirmed values into the
//! `model_pricing` table. The database, not this file, is authoritative in production.

use crate::money::MicroCents;
use crate::types::ModelTier;
use std::collections::HashMap;

/// Pricing and capabilities for one model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPricing {
    /// Canonical identifier, always `provider/model`, e.g. `openai/gpt-4o`.
    pub model_id: String,
    pub provider: String,
    pub display_name: String,
    pub tier: ModelTier,
    /// Cost of one million input tokens.
    pub input_per_mtok: MicroCents,
    /// Cost of one million output tokens.
    pub output_per_mtok: MicroCents,
    pub context_window: u32,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub is_active: bool,
    /// Where this price came from and when it was checked.
    pub source: String,
}

impl ModelPricing {
    /// Cost of serving a request with these token counts.
    pub fn cost(&self, input_tokens: u64, output_tokens: u64) -> MicroCents {
        MicroCents::cost_for_tokens(self.input_per_mtok, input_tokens)
            + MicroCents::cost_for_tokens(self.output_per_mtok, output_tokens)
    }

    /// A single comparable price per million tokens, blending input and output 3:1.
    ///
    /// Ranking candidates needs one number, but models differ in the *ratio* between
    /// input and output prices — some cheap-input models have punishing output prices, and
    /// ranking on input alone would route to them wrongly. Chat traffic runs roughly three
    /// input tokens per output token, so that is the weighting used here.
    pub fn blended_per_mtok(&self) -> MicroCents {
        MicroCents((self.input_per_mtok.0.saturating_mul(3) + self.output_per_mtok.0) / 4)
    }

    /// The bare model name without the provider prefix (`openai/gpt-4o` -> `gpt-4o`).
    pub fn bare_name(&self) -> &str {
        self.model_id
            .split_once('/')
            .map(|(_, n)| n)
            .unwrap_or(&self.model_id)
    }
}

/// What a request needs a model to be able to do. Used to filter routing candidates so
/// we never downgrade into a model that cannot serve the request at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Requirements {
    pub tools: bool,
    pub vision: bool,
    /// Minimum usable context window in tokens.
    pub min_context: u32,
}

/// An indexed, immutable pricing table.
#[derive(Debug, Clone, Default)]
pub struct PricingTable {
    by_canonical: HashMap<String, ModelPricing>,
    /// Alias -> canonical id. Callers say `gpt-4o`; we resolve to `openai/gpt-4o`.
    aliases: HashMap<String, String>,
}

impl PricingTable {
    /// An empty table.
    pub fn new() -> PricingTable {
        PricingTable::default()
    }

    /// Build from rows (from the database in production, from seed data in development).
    pub fn from_models(models: Vec<ModelPricing>) -> PricingTable {
        let mut table = PricingTable::default();
        for model in models {
            table.insert(model);
        }
        table
    }

    /// Insert or replace a model, registering its aliases.
    pub fn insert(&mut self, model: ModelPricing) {
        let canonical = model.model_id.clone();
        let bare = model.bare_name().to_string();

        // The bare name resolves to this model unless another provider already claimed
        // it. First registration wins, which keeps `gpt-4o` pointing at OpenAI even after
        // an aggregator that also serves it is added.
        self.aliases
            .entry(bare)
            .or_insert_with(|| canonical.clone());
        self.aliases.insert(canonical.clone(), canonical.clone());
        self.by_canonical.insert(canonical, model);
    }

    /// Register an extra alias, e.g. a dated snapshot name.
    pub fn add_alias(&mut self, alias: &str, canonical: &str) {
        if self.by_canonical.contains_key(canonical) {
            self.aliases
                .insert(alias.to_string(), canonical.to_string());
        }
    }

    /// Resolve any name a caller might use to a canonical model id.
    ///
    /// Handles exact matches, bare names, and dated snapshots such as
    /// `gpt-4o-2024-11-20`, which providers issue continuously and which would otherwise
    /// each need a table entry.
    pub fn resolve(&self, requested: &str) -> Option<&str> {
        let normalized = requested.trim().to_ascii_lowercase();
        if let Some(canonical) = self.aliases.get(&normalized) {
            return Some(canonical.as_str());
        }
        // Dated snapshot: strip a trailing -YYYY-MM-DD and retry.
        if let Some(base) = strip_date_suffix(&normalized) {
            if let Some(canonical) = self.aliases.get(base) {
                return Some(canonical.as_str());
            }
        }
        None
    }

    /// Look up pricing for any caller-supplied model name.
    pub fn get(&self, requested: &str) -> Option<&ModelPricing> {
        let canonical = self.resolve(requested)?;
        self.by_canonical.get(canonical)
    }

    /// Cost of serving a request on `model`. Returns `None` for unknown models rather
    /// than guessing — an invented price is worse than an absent one.
    pub fn cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<MicroCents> {
        self.get(model).map(|m| m.cost(input_tokens, output_tokens))
    }

    /// Every active model.
    pub fn all(&self) -> impl Iterator<Item = &ModelPricing> {
        self.by_canonical.values().filter(|m| m.is_active)
    }

    /// Number of models in the table, including inactive ones.
    pub fn len(&self) -> usize {
        self.by_canonical.len()
    }

    /// True when the table holds no models.
    pub fn is_empty(&self) -> bool {
        self.by_canonical.is_empty()
    }

    /// True when a model can satisfy the given requirements.
    pub fn satisfies(model: &ModelPricing, requirements: Requirements) -> bool {
        (!requirements.tools || model.supports_tools)
            && (!requirements.vision || model.supports_vision)
            && model.context_window >= requirements.min_context
    }

    /// Candidate models strictly cheaper than `requested`, capable of the same request,
    /// ordered cheapest first.
    ///
    /// Only strictly-cheaper models are returned: routing to something equally priced adds
    /// risk with no upside.
    pub fn cheaper_alternatives(
        &self,
        requested: &str,
        requirements: Requirements,
    ) -> Vec<&ModelPricing> {
        let Some(target) = self.get(requested) else {
            return Vec::new();
        };
        let target_price = target.blended_per_mtok();

        let mut candidates: Vec<&ModelPricing> = self
            .all()
            .filter(|m| m.model_id != target.model_id)
            .filter(|m| m.blended_per_mtok() < target_price)
            .filter(|m| PricingTable::satisfies(m, requirements))
            .collect();

        candidates.sort_by(|a, b| {
            a.blended_per_mtok()
                .cmp(&b.blended_per_mtok())
                // Stable tie-break so routing is deterministic and reproducible from a
                // usage record.
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        candidates
    }

    /// The cheapest capable model at or below `max_tier`.
    pub fn cheapest_at_or_below(
        &self,
        max_tier: ModelTier,
        requirements: Requirements,
    ) -> Option<&ModelPricing> {
        self.all()
            .filter(|m| m.tier <= max_tier)
            .filter(|m| PricingTable::satisfies(m, requirements))
            .min_by(|a, b| {
                a.blended_per_mtok()
                    .cmp(&b.blended_per_mtok())
                    .then_with(|| a.model_id.cmp(&b.model_id))
            })
    }

    /// The cheapest embedding model, used by the semantic cache.
    pub fn cheapest_embedding_model(&self) -> Option<&ModelPricing> {
        self.all()
            .filter(|m| m.model_id.contains("embedding"))
            .min_by_key(|m| m.input_per_mtok)
    }

    /// Development seed data.
    ///
    /// **Verify before billing.** See the module documentation and
    /// `docs/runbooks/pricing-update.md`. Prices are USD per million tokens as published
    /// on the stated date; providers change them without notice.
    pub fn with_seed_data() -> PricingTable {
        let s =
            |provider: &str, date: &str| format!("{provider} published pricing — checked {date}");

        let models = vec![
            // ---------------- OpenAI ----------------
            model(
                "openai/gpt-5",
                "openai",
                "GPT-5",
                ModelTier::Frontier,
                1.25,
                10.00,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-5-mini",
                "openai",
                "GPT-5 mini",
                ModelTier::Mid,
                0.25,
                2.00,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-5-nano",
                "openai",
                "GPT-5 nano",
                ModelTier::Cheap,
                0.05,
                0.40,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-4o",
                "openai",
                "GPT-4o",
                ModelTier::Premium,
                2.50,
                10.00,
                128_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-4o-mini",
                "openai",
                "GPT-4o mini",
                ModelTier::Cheap,
                0.15,
                0.60,
                128_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-4.1",
                "openai",
                "GPT-4.1",
                ModelTier::Premium,
                2.00,
                8.00,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-4.1-mini",
                "openai",
                "GPT-4.1 mini",
                ModelTier::Mid,
                0.40,
                1.60,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/gpt-4.1-nano",
                "openai",
                "GPT-4.1 nano",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/o3",
                "openai",
                "o3",
                ModelTier::Frontier,
                2.00,
                8.00,
                200_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/o4-mini",
                "openai",
                "o4-mini",
                ModelTier::Premium,
                1.10,
                4.40,
                200_000,
                true,
                true,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/text-embedding-3-small",
                "openai",
                "Embedding 3 small",
                ModelTier::Cheap,
                0.02,
                0.0,
                8_191,
                false,
                false,
                s("OpenAI", "2026-08-20"),
            ),
            model(
                "openai/text-embedding-3-large",
                "openai",
                "Embedding 3 large",
                ModelTier::Cheap,
                0.13,
                0.0,
                8_191,
                false,
                false,
                s("OpenAI", "2026-08-20"),
            ),
            // ---------------- Anthropic ----------------
            model(
                "anthropic/claude-opus-4-5",
                "anthropic",
                "Claude Opus 4.5",
                ModelTier::Frontier,
                5.00,
                25.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-20"),
            ),
            model(
                "anthropic/claude-sonnet-4-5",
                "anthropic",
                "Claude Sonnet 4.5",
                ModelTier::Premium,
                3.00,
                15.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-20"),
            ),
            model(
                "anthropic/claude-haiku-4-5",
                "anthropic",
                "Claude Haiku 4.5",
                ModelTier::Mid,
                1.00,
                5.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-20"),
            ),
            model(
                "anthropic/claude-opus-4-1",
                "anthropic",
                "Claude Opus 4.1",
                ModelTier::Frontier,
                15.00,
                75.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-20"),
            ),
            model(
                "anthropic/claude-3-5-haiku",
                "anthropic",
                "Claude 3.5 Haiku",
                ModelTier::Cheap,
                0.80,
                4.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-20"),
            ),
            // ---------------- Google ----------------
            model(
                "google/gemini-2.5-pro",
                "google",
                "Gemini 2.5 Pro",
                ModelTier::Premium,
                1.25,
                10.00,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-20"),
            ),
            model(
                "google/gemini-2.5-flash",
                "google",
                "Gemini 2.5 Flash",
                ModelTier::Mid,
                0.30,
                2.50,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-20"),
            ),
            model(
                "google/gemini-2.5-flash-lite",
                "google",
                "Gemini 2.5 Flash Lite",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-20"),
            ),
            model(
                "google/gemini-2.0-flash",
                "google",
                "Gemini 2.0 Flash",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-20"),
            ),
            // ---------------- DeepSeek ----------------
            model(
                "deepseek/deepseek-chat",
                "deepseek",
                "DeepSeek Chat",
                ModelTier::Cheap,
                0.27,
                1.10,
                128_000,
                true,
                false,
                s("DeepSeek", "2026-08-20"),
            ),
            model(
                "deepseek/deepseek-reasoner",
                "deepseek",
                "DeepSeek Reasoner",
                ModelTier::Mid,
                0.55,
                2.19,
                128_000,
                true,
                false,
                s("DeepSeek", "2026-08-20"),
            ),
            // ---------------- Mistral ----------------
            model(
                "mistral/mistral-large-latest",
                "mistral",
                "Mistral Large",
                ModelTier::Premium,
                2.00,
                6.00,
                131_000,
                true,
                false,
                s("Mistral", "2026-08-20"),
            ),
            model(
                "mistral/mistral-small-latest",
                "mistral",
                "Mistral Small",
                ModelTier::Cheap,
                0.20,
                0.60,
                131_000,
                true,
                false,
                s("Mistral", "2026-08-20"),
            ),
            // ---------------- Groq ----------------
            model(
                "groq/llama-3.3-70b-versatile",
                "groq",
                "Llama 3.3 70B (Groq)",
                ModelTier::Mid,
                0.59,
                0.79,
                131_000,
                true,
                false,
                s("Groq", "2026-08-20"),
            ),
            model(
                "groq/llama-3.1-8b-instant",
                "groq",
                "Llama 3.1 8B (Groq)",
                ModelTier::Cheap,
                0.05,
                0.08,
                131_000,
                true,
                false,
                s("Groq", "2026-08-20"),
            ),
            // ---------------- Moonshot ----------------
            model(
                "moonshot/kimi-k2",
                "moonshot",
                "Kimi K2",
                ModelTier::Mid,
                0.60,
                2.50,
                128_000,
                true,
                false,
                s("Moonshot", "2026-08-20"),
            ),
        ];

        let mut table = PricingTable::from_models(models);

        // Common aliases callers actually send.
        table.add_alias("gpt-4o-latest", "openai/gpt-4o");
        table.add_alias("chatgpt-4o-latest", "openai/gpt-4o");
        table.add_alias("claude-3-5-sonnet", "anthropic/claude-sonnet-4-5");
        table.add_alias("claude-sonnet-4-5-20250929", "anthropic/claude-sonnet-4-5");
        table.add_alias("claude-opus-4-5-20251101", "anthropic/claude-opus-4-5");
        table.add_alias("gemini-flash", "google/gemini-2.5-flash");
        table
    }
}

#[allow(clippy::too_many_arguments)]
fn model(
    id: &str,
    provider: &str,
    display: &str,
    tier: ModelTier,
    input_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
    context_window: u32,
    supports_tools: bool,
    supports_vision: bool,
    source: String,
) -> ModelPricing {
    ModelPricing {
        model_id: id.to_string(),
        provider: provider.to_string(),
        display_name: display.to_string(),
        tier,
        input_per_mtok: MicroCents::from_usd_per_mtok(input_usd_per_mtok),
        output_per_mtok: MicroCents::from_usd_per_mtok(output_usd_per_mtok),
        context_window,
        supports_tools,
        supports_vision,
        is_active: true,
        source,
    }
}

/// Strip a trailing `-YYYY-MM-DD` snapshot suffix, if present.
fn strip_date_suffix(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 11 {
        return None;
    }
    let (base, suffix) = name.split_at(bytes.len() - 11);
    let mut chars = suffix.chars();
    if chars.next() != Some('-') {
        return None;
    }
    let digits: Vec<char> = chars.collect();
    // Expect DDDD-DD-DD.
    let shape_ok = digits.len() == 10
        && digits[0..4].iter().all(|c| c.is_ascii_digit())
        && digits[4] == '-'
        && digits[5..7].iter().all(|c| c.is_ascii_digit())
        && digits[7] == '-'
        && digits[8..10].iter().all(|c| c.is_ascii_digit());
    shape_ok.then_some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> PricingTable {
        PricingTable::with_seed_data()
    }

    #[test]
    fn seed_data_is_populated_and_active() {
        let t = table();
        assert!(
            t.len() >= 20,
            "Phase 0 requires at least 20 seeded models, got {}",
            t.len()
        );
        assert!(t.all().count() >= 20);
        assert!(!t.is_empty());
    }

    #[test]
    fn every_seeded_price_has_dated_provenance() {
        // Part 13 item 8: every cost number traceable to a dated source.
        for m in table().all() {
            assert!(!m.source.is_empty(), "{} has no source", m.model_id);
            assert!(
                m.source.contains("checked"),
                "{} source lacks a date: {}",
                m.model_id,
                m.source
            );
        }
    }

    #[test]
    fn every_model_id_is_provider_qualified() {
        for m in table().all() {
            assert!(
                m.model_id.contains('/'),
                "{} is not provider-qualified",
                m.model_id
            );
            assert!(
                m.model_id.starts_with(&format!("{}/", m.provider)),
                "{}",
                m.model_id
            );
        }
    }

    #[test]
    fn output_is_never_cheaper_than_input_for_chat_models() {
        // A sanity check on data entry: every chat provider charges more for output.
        // Embeddings have no output price, so they are excluded.
        for m in table().all().filter(|m| !m.model_id.contains("embedding")) {
            assert!(
                m.output_per_mtok >= m.input_per_mtok,
                "{} has output ({}) cheaper than input ({}) — likely a transposed price",
                m.model_id,
                m.output_per_mtok,
                m.input_per_mtok
            );
        }
    }

    #[test]
    fn bare_names_resolve_to_canonical_ids() {
        let t = table();
        assert_eq!(t.resolve("gpt-4o"), Some("openai/gpt-4o"));
        assert_eq!(t.resolve("openai/gpt-4o"), Some("openai/gpt-4o"));
        assert_eq!(t.resolve("GPT-4O"), Some("openai/gpt-4o"));
        assert_eq!(t.resolve("  gpt-4o  "), Some("openai/gpt-4o"));
        assert_eq!(t.resolve("no-such-model"), None);
    }

    #[test]
    fn dated_snapshots_resolve_to_the_base_model() {
        // Providers ship new dated snapshots constantly; each must not need a table row.
        let t = table();
        assert_eq!(t.resolve("gpt-4o-2024-11-20"), Some("openai/gpt-4o"));
        assert_eq!(
            t.resolve("gpt-4o-mini-2024-07-18"),
            Some("openai/gpt-4o-mini")
        );
    }

    #[test]
    fn date_suffix_stripping_is_precise() {
        assert_eq!(strip_date_suffix("gpt-4o-2024-11-20"), Some("gpt-4o"));
        assert_eq!(strip_date_suffix("gpt-4o"), None);
        assert_eq!(strip_date_suffix("claude-3-5-haiku"), None);
        assert_eq!(strip_date_suffix("model-20241120"), None);
        assert_eq!(strip_date_suffix("short"), None);
    }

    #[test]
    fn explicit_aliases_resolve() {
        let t = table();
        assert_eq!(t.resolve("chatgpt-4o-latest"), Some("openai/gpt-4o"));
        assert_eq!(
            t.resolve("claude-sonnet-4-5-20250929"),
            Some("anthropic/claude-sonnet-4-5")
        );
    }

    #[test]
    fn cost_matches_hand_calculation() {
        let t = table();
        // GPT-4o at $2.50/$10.00 per Mtok: 1000 in + 500 out
        //   input  = 2.50 * 1000/1e6  = $0.0025  = 2_500 micro-cents
        //   output = 10.00 * 500/1e6  = $0.005   = 5_000 micro-cents
        let cost = t.cost("gpt-4o", 1_000, 500).unwrap();
        assert_eq!(cost, MicroCents(7_500));
        assert_eq!(cost.to_usd_string(), "$0.007500");
    }

    #[test]
    fn unknown_models_have_no_price_rather_than_a_guessed_one() {
        assert_eq!(table().cost("totally-made-up-model", 100, 100), None);
    }

    #[test]
    fn blended_price_weights_input_three_to_one() {
        let m = model(
            "test/m",
            "test",
            "M",
            ModelTier::Mid,
            4.0,
            8.0,
            1000,
            false,
            false,
            "test".into(),
        );
        // (3*4.00 + 8.00) / 4 = $5.00 per Mtok
        assert_eq!(m.blended_per_mtok(), MicroCents::from_usd_per_mtok(5.0));
    }

    #[test]
    fn cheaper_alternatives_are_strictly_cheaper_and_sorted() {
        let t = table();
        let alternatives = t.cheaper_alternatives("gpt-4o", Requirements::default());
        assert!(!alternatives.is_empty());

        let target = t.get("gpt-4o").unwrap().blended_per_mtok();
        let mut previous = MicroCents::ZERO;
        for candidate in &alternatives {
            assert!(
                candidate.blended_per_mtok() < target,
                "{} is not cheaper than gpt-4o",
                candidate.model_id
            );
            assert!(
                candidate.blended_per_mtok() >= previous,
                "not sorted ascending"
            );
            previous = candidate.blended_per_mtok();
        }
        // gpt-4o-mini is the canonical cheap substitute and must be present.
        assert!(alternatives
            .iter()
            .any(|m| m.model_id == "openai/gpt-4o-mini"));
    }

    #[test]
    fn cheaper_alternatives_respect_capability_requirements() {
        let t = table();
        let vision_needed = Requirements {
            vision: true,
            ..Default::default()
        };
        for candidate in t.cheaper_alternatives("openai/gpt-5", vision_needed) {
            assert!(
                candidate.supports_vision,
                "{} cannot do vision",
                candidate.model_id
            );
        }

        let huge_context = Requirements {
            min_context: 500_000,
            ..Default::default()
        };
        for candidate in t.cheaper_alternatives("openai/gpt-5", huge_context) {
            assert!(
                candidate.context_window >= 500_000,
                "{}",
                candidate.model_id
            );
        }
    }

    #[test]
    fn a_request_needing_tools_never_routes_to_a_toolless_model() {
        let t = table();
        let needs_tools = Requirements {
            tools: true,
            ..Default::default()
        };
        for candidate in t.cheaper_alternatives("gpt-4o", needs_tools) {
            assert!(
                candidate.supports_tools,
                "{} lacks tool support",
                candidate.model_id
            );
        }
    }

    #[test]
    fn unknown_model_yields_no_alternatives_rather_than_all_of_them() {
        // A bug here would silently reroute an unrecognised model to the cheapest thing
        // in the table — exactly the quality failure Part 5 warns against.
        assert!(table()
            .cheaper_alternatives("unknown-model", Requirements::default())
            .is_empty());
    }

    #[test]
    fn cheapest_at_or_below_respects_the_tier_ceiling() {
        let t = table();
        let cheap = t
            .cheapest_at_or_below(ModelTier::Cheap, Requirements::default())
            .unwrap();
        assert_eq!(cheap.tier, ModelTier::Cheap);

        let mid = t
            .cheapest_at_or_below(ModelTier::Mid, Requirements::default())
            .unwrap();
        assert!(mid.tier <= ModelTier::Mid);
        // Allowing a higher ceiling can only ever be at least as cheap.
        assert!(mid.blended_per_mtok() <= cheap.blended_per_mtok());
    }

    #[test]
    fn embedding_model_selection_picks_the_cheapest() {
        let t = table();
        let embedding = t.cheapest_embedding_model().unwrap();
        assert_eq!(embedding.model_id, "openai/text-embedding-3-small");
    }

    #[test]
    fn routing_is_deterministic_across_repeated_calls() {
        // Two identical requests must produce the same candidate order, or a usage record
        // cannot be used to explain a routing decision after the fact.
        let t = table();
        let first: Vec<String> = t
            .cheaper_alternatives("gpt-4o", Requirements::default())
            .iter()
            .map(|m| m.model_id.clone())
            .collect();
        for _ in 0..5 {
            let again: Vec<String> = t
                .cheaper_alternatives("gpt-4o", Requirements::default())
                .iter()
                .map(|m| m.model_id.clone())
                .collect();
            assert_eq!(first, again);
        }
    }

    #[test]
    fn zero_token_requests_cost_nothing() {
        assert_eq!(table().cost("gpt-4o", 0, 0), Some(MicroCents::ZERO));
    }

    #[test]
    fn large_requests_do_not_overflow() {
        // A million-token context against the priciest model must still produce a sane
        // number rather than wrapping into a negative charge.
        let cost = table()
            .cost("anthropic/claude-opus-4-1", 1_000_000, 1_000_000)
            .unwrap();
        assert!(cost > MicroCents::ZERO);
        // $15 + $75 = $90.00 = 90_000_000 micro-cents
        assert_eq!(cost, MicroCents(90_000_000));
    }
}
