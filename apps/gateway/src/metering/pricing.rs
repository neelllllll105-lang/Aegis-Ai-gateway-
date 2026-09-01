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

/// Marker written into `source` for any price that has not been re-verified.
///
/// The admin console and `docs/runbooks/pricing-update.md` both search for this string,
/// so a row carrying it is impossible to lose track of.
pub const UNVERIFIED: &str =
    "UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md";

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
    /// Whether this model can serve a chat completion at all.
    ///
    /// **False for embedding models**, and that distinction is load-bearing rather than
    /// cosmetic. An embedding model is the cheapest thing in the table by a wide margin
    /// ($0.02/Mtok input, $0.00 output), reports `supports_tools: false` and
    /// `supports_vision: false`, and therefore satisfied every capability filter a plain
    /// chat request imposed — so it won the price sort and *every simple chat request was
    /// being routed to it*. It cannot answer a chat request at all; the provider would
    /// have returned an error or an embedding vector.
    ///
    /// The router's own test did not catch this because it asserted only that the served
    /// model was not the requested one. Capability filtering has to include "can it do the
    /// kind of work being asked", not just "does it have the features being used".
    pub supports_chat: bool,
    pub is_active: bool,
    /// Where this price came from and when it was checked.
    pub source: String,
    /// How this model prices prompt-cache reads and writes.
    ///
    /// Defaults to [`CachePricing::none`], which bills cached tokens at the full input
    /// rate — the correct behaviour for a model with no prompt cache, and a deliberately
    /// conservative default for one whose cache rates have not been verified.
    pub cache: CachePricing,
    /// A higher rate that applies once a prompt passes a token threshold.
    ///
    /// `None` for the flat-priced majority. Gemini 2.5 Pro doubles its input rate and
    /// raises its output rate by half past 200,000 tokens, against a context window of
    /// 1,048,576 — so a flat price under-bills any long-context request by a wide margin
    /// on a model explicitly sold for long context. Found in the enterprise readiness
    /// audit.
    pub long_context: Option<LongContextTier>,
}

/// Prompt-cache rates, as a proportion of the model's own input rate.
///
/// Stored as basis points rather than a float, for the same reason every other money
/// figure here is an integer: a rate that drifts by a rounding error produces invoices
/// that do not reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePricing {
    /// Rate for a cache *read*, in basis points of the input rate. 1,000 bp = 10%.
    pub read_bp: u32,
    /// Rate for a cache *write*, in basis points of the input rate. 12,500 bp = 125%.
    pub write_bp: u32,
}

impl CachePricing {
    /// No prompt cache: reads and writes both bill at the ordinary input rate.
    pub const fn none() -> CachePricing {
        CachePricing {
            read_bp: 10_000,
            write_bp: 10_000,
        }
    }

    /// OpenAI and Google: cache reads at 25% of input, no separate write charge.
    pub const fn quarter_read() -> CachePricing {
        CachePricing {
            read_bp: 2_500,
            write_bp: 10_000,
        }
    }

    /// Anthropic: reads at 10% of input, writes at 125% (a five-minute cache entry).
    pub const fn anthropic() -> CachePricing {
        CachePricing {
            read_bp: 1_000,
            write_bp: 12_500,
        }
    }

    /// Apply a basis-point rate to a per-million-token price.
    fn scale(rate: MicroCents, bp: u32) -> MicroCents {
        MicroCents(rate.0.saturating_mul(bp as i64) / 10_000)
    }
}

impl Default for CachePricing {
    fn default() -> CachePricing {
        CachePricing::none()
    }
}

/// A second, higher price that applies to prompts past a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongContextTier {
    /// Total input tokens at or above which the higher rates apply.
    pub threshold_tokens: u64,
    pub input_per_mtok: MicroCents,
    pub output_per_mtok: MicroCents,
}

impl ModelPricing {
    /// Cost of serving a request with these token counts.
    ///
    /// Kept for the many callers that only have two figures — a projection, a baseline
    /// comparison, a test. Requests actually served go through [`ModelPricing::cost_of`],
    /// which prices cached tokens at their real rate.
    pub fn cost(&self, input_tokens: u64, output_tokens: u64) -> MicroCents {
        self.cost_of(&crate::types::TokenUsage {
            input_tokens,
            output_tokens,
            ..Default::default()
        })
    }

    /// Cost of serving a request, across every token class this model prices separately.
    ///
    /// The rates used depend on total prompt size when the model has a long-context tier,
    /// which is why this takes the whole usage rather than one figure at a time.
    pub fn cost_of(&self, usage: &crate::types::TokenUsage) -> MicroCents {
        let (input_rate, output_rate) = self.rates_for(usage.total_input());

        MicroCents::cost_for_tokens(input_rate, usage.input_tokens)
            + MicroCents::cost_for_tokens(output_rate, usage.output_tokens)
            + MicroCents::cost_for_tokens(
                CachePricing::scale(input_rate, self.cache.read_bp),
                usage.cached_input_tokens,
            )
            + MicroCents::cost_for_tokens(
                CachePricing::scale(input_rate, self.cache.write_bp),
                usage.cache_write_tokens,
            )
    }

    /// The input and output rates that apply to a prompt of this size.
    pub fn rates_for(&self, input_tokens: u64) -> (MicroCents, MicroCents) {
        match self.long_context {
            Some(tier) if input_tokens >= tier.threshold_tokens => {
                (tier.input_per_mtok, tier.output_per_mtok)
            }
            _ => (self.input_per_mtok, self.output_per_mtok),
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirements {
    pub tools: bool,
    pub vision: bool,
    /// Minimum usable context window in tokens.
    pub min_context: u32,
    /// Whether the candidate must be able to serve a chat completion.
    ///
    /// Defaults to **true**, because that is what the overwhelming majority of traffic
    /// needs and because the failure mode of getting this wrong — routing a chat request
    /// to an embedding model — is far worse than the failure mode of being too strict.
    pub chat: bool,
}

impl Default for Requirements {
    fn default() -> Requirements {
        Requirements {
            tools: false,
            vision: false,
            min_context: 0,
            chat: true,
        }
    }
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

    /// Cost of serving a request, honouring cached tokens and long-context tiers.
    ///
    /// The form the request path uses. [`PricingTable::cost`] remains for callers that
    /// genuinely only have two token counts — a pre-flight projection, a test — but every
    /// figure that reaches an invoice goes through this one.
    pub fn cost_of(&self, model: &str, usage: &crate::types::TokenUsage) -> Option<MicroCents> {
        self.get(model).map(|m| m.cost_of(usage))
    }

    /// What this request *would* have cost on the model the caller asked for.
    ///
    /// The savings baseline. Uses the same token counts, which is the honest comparison:
    /// the question is "what would the same work have cost there", not "what would a
    /// differently-cached version of it have cost".
    pub fn baseline_of(
        &self,
        requested_model: &str,
        usage: &crate::types::TokenUsage,
        fallback: MicroCents,
    ) -> MicroCents {
        self.cost_of(requested_model, usage).unwrap_or(fallback)
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
        (!requirements.chat || model.supports_chat)
            && (!requirements.tools || model.supports_tools)
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
            .filter(|m| !m.supports_chat)
            .min_by_key(|m| m.input_per_mtok)
    }

    /// Development seed data.
    ///
    /// **Verify before billing.** See the module documentation and
    /// `docs/runbooks/pricing-update.md`. Prices are USD per million tokens as published
    /// on the stated date; providers change them without notice.
    pub fn with_seed_data() -> PricingTable {
        let s =
            |provider: &str, date: &str| format!("{provider} published pricing — verified {date}");

        // Verified 2026-08-21 against each published pricing page.
        //
        // Three conservative choices, applied consistently, because every one of these
        // numbers becomes a savings claim on a customer invoice:
        //
        //   * UNCACHED input rates. Assuming a cache discount we do not always receive
        //     would understate cost and therefore overstate savings.
        //   * BASE context tier. Gemini 2.5 Pro charges more above 200k tokens; quoting
        //     the base rate means long-context requests cost us margin, not the customer.
        //   * PEAK rates for DeepSeek, which halves prices off-peak. Billing the off-peak
        //     rate for a peak-hour request would under-state what the request really cost.
        let models = vec![
            // ---------------- OpenAI ----------------
            // developers.openai.com/api/docs/pricing
            m(
                "openai/gpt-5",
                "openai",
                "GPT-5",
                ModelTier::Frontier,
                1.25,
                10.00,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-5-mini",
                "openai",
                "GPT-5 mini",
                ModelTier::Mid,
                0.25,
                2.00,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-5-nano",
                "openai",
                "GPT-5 nano",
                ModelTier::Cheap,
                0.05,
                0.40,
                400_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-4o",
                "openai",
                "GPT-4o",
                ModelTier::Premium,
                2.50,
                10.00,
                128_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-4o-mini",
                "openai",
                "GPT-4o mini",
                ModelTier::Cheap,
                0.15,
                0.60,
                128_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-4.1",
                "openai",
                "GPT-4.1",
                ModelTier::Premium,
                2.00,
                8.00,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-4.1-mini",
                "openai",
                "GPT-4.1 mini",
                ModelTier::Mid,
                0.40,
                1.60,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/gpt-4.1-nano",
                "openai",
                "GPT-4.1 nano",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_047_576,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/o3",
                "openai",
                "o3",
                ModelTier::Frontier,
                2.00,
                8.00,
                200_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/o4-mini",
                "openai",
                "o4-mini",
                ModelTier::Premium,
                1.10,
                4.40,
                200_000,
                true,
                true,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/text-embedding-3-small",
                "openai",
                "Embedding 3 small",
                ModelTier::Cheap,
                0.02,
                0.0,
                8_191,
                false,
                false,
                s("OpenAI", "2026-08-21"),
            ),
            m(
                "openai/text-embedding-3-large",
                "openai",
                "Embedding 3 large",
                ModelTier::Cheap,
                0.13,
                0.0,
                8_191,
                false,
                false,
                s("OpenAI", "2026-08-21"),
            ),
            // ---------------- Anthropic ----------------
            // platform.claude.com/docs/en/about-claude/pricing
            //
            // Sonnet 5 at $2/$10 is cheaper than Sonnet 4.5 at $3/$15, so the router now
            // prefers it — a real saving that exists only because this table is current.
            // That is the case for the monthly pricing runbook in one line.
            m(
                "anthropic/claude-fable-5",
                "anthropic",
                "Claude Fable 5",
                ModelTier::Frontier,
                10.00,
                50.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-opus-5",
                "anthropic",
                "Claude Opus 5",
                ModelTier::Frontier,
                5.00,
                25.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-opus-4-8",
                "anthropic",
                "Claude Opus 4.8",
                ModelTier::Frontier,
                5.00,
                25.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-opus-4-7",
                "anthropic",
                "Claude Opus 4.7",
                ModelTier::Frontier,
                5.00,
                25.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-opus-4-6",
                "anthropic",
                "Claude Opus 4.6",
                ModelTier::Frontier,
                5.00,
                25.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-opus-4-5",
                "anthropic",
                "Claude Opus 4.5",
                ModelTier::Frontier,
                5.00,
                25.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-sonnet-5",
                "anthropic",
                "Claude Sonnet 5",
                ModelTier::Premium,
                2.00,
                10.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-sonnet-4-6",
                "anthropic",
                "Claude Sonnet 4.6",
                ModelTier::Premium,
                3.00,
                15.00,
                1_000_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-sonnet-4-5",
                "anthropic",
                "Claude Sonnet 4.5",
                ModelTier::Premium,
                3.00,
                15.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-haiku-4-5",
                "anthropic",
                "Claude Haiku 4.5",
                ModelTier::Mid,
                1.00,
                5.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            // ---------------- Google ----------------
            // ai.google.dev/gemini-api/docs/pricing — base context tier.
            m(
                "google/gemini-3.6-flash",
                "google",
                "Gemini 3.6 Flash",
                ModelTier::Mid,
                0.30,
                2.50,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-24"),
            ),
            m(
                "google/gemini-3.1-pro-preview",
                "google",
                "Gemini 3.1 Pro Preview",
                ModelTier::Premium,
                1.25,
                10.00,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-24"),
            ),
            m(
                "google/gemini-2.5-pro",
                "google",
                "Gemini 2.5 Pro",
                ModelTier::Premium,
                1.25,
                10.00,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-21"),
            ),
            m(
                "google/gemini-2.5-flash",
                "google",
                "Gemini 2.5 Flash",
                ModelTier::Mid,
                0.30,
                2.50,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-21"),
            ),
            m(
                "google/gemini-2.5-flash-lite",
                "google",
                "Gemini 2.5 Flash Lite",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_048_576,
                true,
                true,
                // Confirmed 2026-08-24 via live search (Google's own pricing page did not
                // render through the fetch tool used — see the audit report). Same call
                // found this model's retirement is announced for 2026-10-16, about seven
                // weeks out at time of writing. Nothing in this pricing table currently
                // tracks upcoming provider deprecations; see the audit's pricing section.
                s("Google", "2026-08-24"),
            ),
            // ---------------- Vertex AI ----------------
            // Google's stated policy is that Vertex charges the identical per-token rate
            // as AI Studio for the same Gemini model — Vertex's price is not for the
            // tokens, it is for the enterprise surface around them (VPC-SC, IAM, no
            // training on your data by default, regional processing guarantees).
            // Confirmed 2026-08-24 via live search against third-party pricing trackers,
            // cross-checked against Google's own numbers already in this file for the
            // `google/` rows above, since the primary pricing page did not render through
            // the fetch tool used.
            //
            // NOT MODELLED, and a real gap: Gemini 2.5 Pro bills prompts over 200K tokens
            // at $2.50 / $15.00 per Mtok instead of the base $1.25 / $10.00 used below.
            // `ModelPricing::cost()` has no concept of a context-length-dependent tier —
            // every request is priced at the flat per-model rate regardless of prompt
            // size. A request to gemini-2.5-pro (context window 1,048,576 tokens) with a
            // prompt anywhere past 200K is under-billed by this gateway relative to what
            // Google actually charges the organisation's own provider account under BYOK.
            // See the pricing/metering section of the enterprise audit for the fix this
            // needs; not attempted here because it changes `ModelPricing`'s shape for
            // every provider, not just Google's.
            m(
                "vertex/gemini-2.5-pro",
                "vertex",
                "Gemini 2.5 Pro (Vertex AI)",
                ModelTier::Premium,
                1.25,
                10.00,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-24"),
            ),
            m(
                "vertex/gemini-2.5-flash",
                "vertex",
                "Gemini 2.5 Flash (Vertex AI)",
                ModelTier::Mid,
                0.30,
                2.50,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-24"),
            ),
            m(
                "vertex/gemini-2.5-flash-lite",
                "vertex",
                "Gemini 2.5 Flash Lite (Vertex AI)",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-24"),
            ),
            m(
                "vertex/gemini-3.6-flash",
                "vertex",
                "Gemini 3.6 Flash (Vertex AI)",
                ModelTier::Mid,
                0.30,
                2.50,
                1_048_576,
                true,
                true,
                s("Google (Vertex parity)", "2026-09-01"),
            ),
            m(
                "vertex/gemini-3.1-pro-preview",
                "vertex",
                "Gemini 3.1 Pro Preview (Vertex AI)",
                ModelTier::Premium,
                1.25,
                10.00,
                1_048_576,
                true,
                true,
                s("Google (Vertex parity)", "2026-09-01"),
            ),
            // ---------------- DeepSeek ----------------
            // api-docs.deepseek.com — PEAK rates, see the note above.
            m(
                "deepseek/deepseek-v4-flash",
                "deepseek",
                "DeepSeek V4 Flash",
                ModelTier::Cheap,
                0.44,
                1.32,
                1_000_000,
                true,
                false,
                s("DeepSeek", "2026-08-21"),
            ),
            m(
                "deepseek/deepseek-v4-pro",
                "deepseek",
                "DeepSeek V4 Pro",
                ModelTier::Mid,
                1.32,
                3.96,
                1_000_000,
                true,
                false,
                s("DeepSeek", "2026-08-21"),
            ),
            // ---------------- Mistral / Groq / Moonshot ----------------
            // Verified 2026-09-01 against official published pricing pages.
            m(
                "mistral/mistral-large-latest",
                "mistral",
                "Mistral Large",
                ModelTier::Premium,
                0.50,
                1.50,
                131_072,
                true,
                false,
                s("Mistral", "2026-09-01"),
            ),
            m(
                "mistral/mistral-small-latest",
                "mistral",
                "Mistral Small",
                ModelTier::Cheap,
                0.15,
                0.60,
                131_072,
                true,
                false,
                s("Mistral", "2026-09-01"),
            ),
            m(
                "mistral/mistral-embed",
                "mistral",
                "Mistral Embed",
                ModelTier::Cheap,
                0.10,
                0.0,
                8_192,
                false,
                false,
                s("Mistral", "2026-09-01"),
            ),
            m(
                "groq/llama-3.3-70b-versatile",
                "groq",
                "Llama 3.3 70B (Groq)",
                ModelTier::Mid,
                0.59,
                0.79,
                131_072,
                true,
                false,
                s("Groq", "2026-09-01"),
            ),
            m(
                "groq/llama-3.1-8b-instant",
                "groq",
                "Llama 3.1 8B (Groq)",
                ModelTier::Cheap,
                0.05,
                0.08,
                131_072,
                true,
                false,
                s("Groq", "2026-09-01"),
            ),
            m(
                "moonshot/kimi-k2",
                "moonshot",
                "Kimi K2",
                ModelTier::Mid,
                0.60,
                2.50,
                128_000,
                true,
                false,
                s("Moonshot", "2026-09-01"),
            ),
        ];

        // ---- Prompt-cache and long-context rates ------------------------------------
        //
        // Applied here rather than as ten more positional arguments on `m()`, which is
        // already at the limit of what a reader can follow. Every model from a provider
        // with a prompt cache gets that provider's rates; everything else keeps the
        // conservative default of no discount.
        //
        // These are *proportions* of each model's own input rate, not absolute prices,
        // which is why one line covers a whole provider and why a base-rate change cannot
        // leave the cache rate stale behind it.
        let models: Vec<ModelPricing> = models
            .into_iter()
            .map(|model| {
                let cache = match model.provider.as_str() {
                    // Anthropic: cache reads at 10% of input, writes at 125% (the cost of
                    // populating a five-minute cache entry). Verified against Anthropic's
                    // prompt-caching pricing, 2026-08-26.
                    "anthropic" => CachePricing::anthropic(),
                    // OpenAI, Google AI Studio, and Vertex all discount a cache read to
                    // roughly a quarter of the input rate and do not charge per write.
                    // Verified 2026-08-26.
                    "openai" | "google" | "vertex" => CachePricing::quarter_read(),
                    // Everything else: no known prompt cache, or rates not verified. The
                    // default bills cached tokens at full price, which can only over-state
                    // our own cost estimate, never under-state a customer's bill.
                    _ => CachePricing::none(),
                };
                // An embedding model has no prompt cache regardless of provider.
                if model.model_id.contains("embed") {
                    return with_cache(model, CachePricing::none());
                }
                with_cache(model, cache)
            })
            .map(|model| {
                // Gemini 2.5 Pro is the one model in this table with a genuine second
                // pricing tier: past 200,000 input tokens the input rate doubles and the
                // output rate rises by half, against a 1,048,576-token context window.
                // A flat price under-bills every long-context request on a model sold
                // specifically for long context. Verified 2026-08-26.
                if model.model_id.ends_with("/gemini-2.5-pro") {
                    return with_long_context(model, 200_000, 2.50, 15.00);
                }
                model
            })
            .collect();

        let mut table = PricingTable::from_models(models);

        // Retired and deprecated models. Retained but INACTIVE, so a historical usage
        // record can still be priced while the router will never select one.
        for retired in [
            m(
                "anthropic/claude-opus-4-1",
                "anthropic",
                "Claude Opus 4.1 (retired)",
                ModelTier::Frontier,
                15.00,
                75.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "anthropic/claude-3-5-haiku",
                "anthropic",
                "Claude 3.5 Haiku (retired)",
                ModelTier::Cheap,
                0.80,
                4.00,
                200_000,
                true,
                true,
                s("Anthropic", "2026-08-21"),
            ),
            m(
                "google/gemini-2.0-flash",
                "google",
                "Gemini 2.0 Flash (deprecated)",
                ModelTier::Cheap,
                0.10,
                0.40,
                1_048_576,
                true,
                true,
                s("Google", "2026-08-21"),
            ),
            // A later commit added these to the provider adapter's accepted-model list
            // (providers/google.rs) without a matching pricing entry. Every cost lookup
            // in the pipeline does `.unwrap_or(MicroCents::ZERO)` on a miss, so leaving
            // these unpriced meant a real request served by either one would be metered
            // at exactly $0 — understating a customer's baseline cost if requested, and
            // silently under-counting spend against their budget if ever served. Retired
            // rather than active: Google has moved traffic to the 2.x/3.x lineup, and
            // these exist so a historical or backward-compatible request can still be
            // priced, not so the router selects them. UNVERIFIED rather than a
            // remembered number, per this table's own rule that a price is only marked
            // sourced once checked against the provider's current page.
            m(
                "google/gemini-1.5-flash",
                "google",
                "Gemini 1.5 Flash (retired)",
                ModelTier::Cheap,
                0.075,
                0.30,
                1_048_576,
                true,
                true,
                UNVERIFIED.to_string(),
            ),
            m(
                "google/gemini-1.5-pro",
                "google",
                "Gemini 1.5 Pro (retired)",
                ModelTier::Mid,
                1.25,
                5.00,
                2_097_152,
                true,
                true,
                UNVERIFIED.to_string(),
            ),
        ] {
            table.insert(ModelPricing {
                is_active: false,
                ..retired
            });
        }

        // Aliases callers actually send.
        table.add_alias("gpt-4o-latest", "openai/gpt-4o");
        table.add_alias("chatgpt-4o-latest", "openai/gpt-4o");
        table.add_alias("claude-3-5-sonnet", "anthropic/claude-sonnet-4-5");
        table.add_alias("claude-sonnet-4-5-20250929", "anthropic/claude-sonnet-4-5");
        table.add_alias("claude-opus-4-5-20251101", "anthropic/claude-opus-4-5");
        table.add_alias("gemini-flash", "google/gemini-2.5-flash");
        // DeepSeek renamed its whole lineup. These keep an existing integration working
        // instead of failing with an unknown model.
        table.add_alias("deepseek-chat", "deepseek/deepseek-v4-flash");
        table.add_alias("deepseek-reasoner", "deepseek/deepseek-v4-pro");
        table
    }
}

#[allow(clippy::too_many_arguments)]
fn m(
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
        // Every seeded model serves chat except the embedding models, which are corrected
        // below. Detecting by name is what `cheapest_embedding_model` already does; the
        // field makes the property explicit rather than re-deriving it at every call site.
        supports_chat: !id.contains("embed"),
        is_active: true,
        source,
        // Conservative by default: no prompt-cache discount unless a row opts in below.
        // Getting this wrong in the generous direction under-bills silently, so the
        // default is the one that cannot.
        cache: CachePricing::none(),
        long_context: None,
    }
}

/// Attach prompt-cache rates to a seed row.
fn with_cache(model: ModelPricing, cache: CachePricing) -> ModelPricing {
    ModelPricing { cache, ..model }
}

/// Attach a long-context tier to a seed row.
fn with_long_context(
    model: ModelPricing,
    threshold_tokens: u64,
    input_usd_per_mtok: f64,
    output_usd_per_mtok: f64,
) -> ModelPricing {
    ModelPricing {
        long_context: Some(LongContextTier {
            threshold_tokens,
            input_per_mtok: MicroCents::from_usd_per_mtok(input_usd_per_mtok),
            output_per_mtok: MicroCents::from_usd_per_mtok(output_usd_per_mtok),
        }),
        ..model
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

    // ---------------------------------------------------------------------------
    // Prompt-cache and long-context pricing.
    //
    // The gap these close was the largest metering-accuracy finding of the enterprise
    // readiness audit, and it was invisible in both directions at once: Anthropic reports
    // cache tokens additively (so reading only `input_tokens` under-counted) while OpenAI
    // folds them into `prompt_tokens` (so reading it whole over-counted). Every test below
    // pins one half of that.
    // ---------------------------------------------------------------------------

    #[test]
    fn an_anthropic_cache_read_costs_a_tenth_of_full_input() {
        let t = table();
        let model = t.get("anthropic/claude-sonnet-5").expect("seeded");

        let all_fresh = crate::types::TokenUsage {
            input_tokens: 10_000,
            output_tokens: 0,
            ..Default::default()
        };
        let all_cached = crate::types::TokenUsage {
            input_tokens: 0,
            cached_input_tokens: 10_000,
            output_tokens: 0,
            ..Default::default()
        };

        let fresh = model.cost_of(&all_fresh).as_i64();
        let cached = model.cost_of(&all_cached).as_i64();
        assert_eq!(
            cached * 10,
            fresh,
            "a cache read bills at 10% of the input rate"
        );
    }

    #[test]
    fn an_anthropic_cache_write_costs_a_premium() {
        // Writing to the cache is *more* expensive than a fresh token, not less. Treating
        // a cache write as ordinary input under-bills by 25% of the write.
        let t = table();
        let model = t.get("anthropic/claude-sonnet-5").expect("seeded");

        let fresh = model
            .cost_of(&crate::types::TokenUsage {
                input_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();
        let written = model
            .cost_of(&crate::types::TokenUsage {
                cache_write_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();

        assert!(written > fresh, "a cache write costs more than fresh input");
        assert_eq!(written * 100, fresh * 125);
    }

    #[test]
    fn an_openai_cache_read_costs_a_quarter_of_full_input() {
        let t = table();
        let model = t.get("openai/gpt-4o").expect("seeded");

        let fresh = model
            .cost_of(&crate::types::TokenUsage {
                input_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();
        let cached = model
            .cost_of(&crate::types::TokenUsage {
                cached_input_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();

        assert_eq!(cached * 4, fresh);
    }

    #[test]
    fn a_model_with_no_prompt_cache_bills_cached_tokens_at_full_rate() {
        // The conservative default. A provider whose cache rates nobody has verified must
        // not silently receive a discount — that direction under-bills, which is the one
        // that loses money without anyone noticing.
        let t = table();
        let model = t.get("mistral/mistral-large-latest").expect("seeded");

        let fresh = model
            .cost_of(&crate::types::TokenUsage {
                input_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();
        let cached = model
            .cost_of(&crate::types::TokenUsage {
                cached_input_tokens: 10_000,
                ..Default::default()
            })
            .as_i64();

        assert_eq!(cached, fresh);
    }

    #[test]
    fn gemini_two_five_pro_charges_more_past_two_hundred_thousand_tokens() {
        // The tier that did not exist. Gemini 2.5 Pro doubles its input rate past 200k
        // against a 1,048,576-token window, so a flat price under-bills exactly the
        // long-context requests the model is sold for.
        let t = table();
        let model = t.get("google/gemini-2.5-pro").expect("seeded");

        let per_token_below = model
            .cost_of(&crate::types::TokenUsage {
                input_tokens: 199_999,
                ..Default::default()
            })
            .as_i64() as f64
            / 199_999.0;
        let per_token_above = model
            .cost_of(&crate::types::TokenUsage {
                input_tokens: 200_000,
                ..Default::default()
            })
            .as_i64() as f64
            / 200_000.0;

        assert!(
            per_token_above > per_token_below * 1.9,
            "past the threshold the input rate roughly doubles: {per_token_below} -> \
             {per_token_above}"
        );
    }

    #[test]
    fn the_long_context_threshold_counts_every_input_class() {
        // A prompt that is 190k cached plus 20k fresh is a 210k prompt. Counting only the
        // fresh tokens would let a heavily-cached long-context request slip under the
        // threshold and be billed at the base rate.
        let t = table();
        let model = t.get("google/gemini-2.5-pro").expect("seeded");

        let (base_in, _) = model.rates_for(100_000);
        let (tiered_in, _) = model.rates_for(210_000);
        assert!(tiered_in > base_in);

        let usage = crate::types::TokenUsage {
            input_tokens: 20_000,
            cached_input_tokens: 190_000,
            ..Default::default()
        };
        let (applied, _) = model.rates_for(usage.total_input());
        assert_eq!(applied, tiered_in, "210k total input is past the threshold");
    }

    #[test]
    fn a_flat_priced_model_has_no_second_tier() {
        let t = table();
        let model = t.get("openai/gpt-4o").expect("seeded");
        let (small_in, small_out) = model.rates_for(1_000);
        let (huge_in, huge_out) = model.rates_for(10_000_000);
        assert_eq!((small_in, small_out), (huge_in, huge_out));
    }

    #[test]
    fn cost_and_cost_of_agree_when_nothing_is_cached() {
        // `cost` is the two-argument form many callers still use. It must remain exactly
        // equivalent to `cost_of` for a request with no cached tokens, or the projection
        // used by the budget check would disagree with the figure that reaches the invoice.
        let t = table();
        for model in t.all() {
            let via_cost = model.cost(1_234, 567);
            let via_cost_of = model.cost_of(&crate::types::TokenUsage {
                input_tokens: 1_234,
                output_tokens: 567,
                ..Default::default()
            });
            assert_eq!(via_cost, via_cost_of, "{} disagrees", model.model_id);
        }
    }

    #[test]
    fn no_seeded_model_gives_a_cache_discount_it_cannot_justify() {
        // A guard against a future edit adding a discount to a provider that has no
        // prompt cache. Only the three provider families with verified rates may deviate
        // from full price.
        for model in table().all() {
            let discounted = model.cache != CachePricing::none();
            if discounted {
                assert!(
                    matches!(
                        model.provider.as_str(),
                        "anthropic" | "openai" | "google" | "vertex"
                    ),
                    "{} discounts cached tokens but its provider has no verified cache \
                     rates",
                    model.model_id
                );
            }
        }
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
        // Part 13 item 8: every cost number traceable to a dated source. Asserting an
        // actual YYYY-MM-DD rather than a keyword, so a source that merely *sounds*
        // authoritative without saying when it was checked still fails.
        for m in table().all() {
            assert!(!m.source.is_empty(), "{} has no source", m.model_id);

            let has_date = m.source.split_whitespace().any(|word| {
                let bytes = word.as_bytes();
                bytes.len() == 10
                    && bytes[4] == b'-'
                    && bytes[7] == b'-'
                    && word.chars().filter(|c| c.is_ascii_digit()).count() == 8
            });
            let flagged_unverified = m.source.contains("UNVERIFIED");

            assert!(
                has_date || flagged_unverified,
                "{} has neither a date nor an UNVERIFIED marker: {}",
                m.model_id,
                m.source
            );
        }
    }

    #[test]
    fn unverified_prices_are_findable() {
        // The runbook and the admin console both locate outstanding rows by this marker.
        // Once all prices are verified against published provider price sheets,
        // no active unverified rows should remain.
        let t = table();
        let unverified: Vec<&str> = t
            .all()
            .filter(|m| m.source.contains(UNVERIFIED))
            .map(|m| m.model_id.as_str())
            .collect();

        assert_eq!(
            unverified.len(),
            0,
            "expected 0 unverified rows after full verification, found: {unverified:?}"
        );
    }

    #[test]
    fn retired_models_are_priced_but_never_routed_to() {
        // A historical usage record must still price correctly, but the router must not
        // select a model the provider has withdrawn.
        let t = table();

        // Still resolvable and priceable.
        assert!(t.get("anthropic/claude-opus-4-1").is_some());
        assert!(t.cost("anthropic/claude-opus-4-1", 1_000, 500).is_some());

        // But absent from the active set the router draws candidates from.
        assert!(
            !t.all().any(|m| m.model_id == "anthropic/claude-opus-4-1"),
            "a retired model appeared in the active set"
        );
        for candidate in t.cheaper_alternatives("openai/gpt-5", Requirements::default()) {
            assert!(candidate.is_active, "{} is retired", candidate.model_id);
        }
    }

    #[test]
    fn a_newer_cheaper_model_wins_over_its_predecessor() {
        // Sonnet 5 ($2/$10) is cheaper than Sonnet 4.5 ($3/$15). Keeping the table current
        // is what turns that into a real saving, so it is worth asserting directly.
        let t = table();
        let sonnet_5 = t.get("anthropic/claude-sonnet-5").expect("sonnet 5");
        let sonnet_45 = t.get("anthropic/claude-sonnet-4-5").expect("sonnet 4.5");

        assert!(sonnet_5.blended_per_mtok() < sonnet_45.blended_per_mtok());

        let alternatives =
            t.cheaper_alternatives("anthropic/claude-sonnet-4-5", Requirements::default());
        assert!(
            alternatives
                .iter()
                .any(|m| m.model_id == "anthropic/claude-sonnet-5"),
            "Sonnet 5 should be offered as a cheaper alternative to Sonnet 4.5"
        );
    }

    #[test]
    fn renamed_provider_models_still_resolve_through_aliases() {
        // DeepSeek renamed its entire lineup. An existing integration sending the old
        // name must keep working rather than failing with an unknown model.
        let t = table();
        assert_eq!(
            t.resolve("deepseek-chat"),
            Some("deepseek/deepseek-v4-flash")
        );
        assert_eq!(
            t.resolve("deepseek-reasoner"),
            Some("deepseek/deepseek-v4-pro")
        );
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
        for m in table().all().filter(|m| !m.model_id.contains("embed")) {
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
        let m = m(
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
