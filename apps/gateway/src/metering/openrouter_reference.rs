//! OpenRouter pricing as a reference/cross-check source — never the billing source.
//!
//! # Why this exists, and what it is not
//!
//! `metering::pricing` is what Aegis actually bills from, and it is deliberately
//! human-verified: `docs/runbooks/pricing-update.md` explains why a scraped or
//! third-party number never writes into `model_pricing` directly. OpenRouter is a
//! reseller, not the provider itself — their own docs say pricing is "from the top
//! provider for this model" without confirming there is no markup, and Aegis calls
//! providers directly rather than through OpenRouter, so there is no guarantee their
//! number matches what Aegis itself is actually billed.
//!
//! What OpenRouter's `/api/v1/models` *is* good for: it is public, free, structured JSON
//! covering hundreds of models across dozens of providers in one call, which makes it a
//! much better raw signal for "does something look like it changed" than scraping
//! marketing-page HTML ever was. This module fetches it and stores it in its own
//! `openrouter_pricing_reference` table — a separate table from `model_pricing`, on
//! purpose, so it is structurally impossible for this data to be read by the request
//! path or the router. Comparing it against `model_pricing` to actually flag drift is
//! the natural next step and is not built yet.

use crate::error::{AegisError, Result};
use crate::money::MicroCents;
use serde::Deserialize;

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Debug, Clone, Deserialize)]
struct ListResponse {
    data: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct Model {
    id: String,
    name: String,
    #[serde(default)]
    context_length: Option<i64>,
    #[serde(default)]
    pricing: Pricing,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Pricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
}

/// One model, ready to upsert into `openrouter_pricing_reference`.
///
/// Every `*_per_mtok_mc` field is `None` when OpenRouter did not report that price at
/// all, and `Some(0)` when it reported an actual zero (a genuinely free model, or a cache
/// tier some providers don't charge for). Those are different facts and this type keeps
/// them different rather than collapsing "unknown" into "free".
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterPricingRow {
    pub model_id: String,
    pub display_name: String,
    pub context_length: Option<i64>,
    pub input_per_mtok_mc: Option<i64>,
    pub output_per_mtok_mc: Option<i64>,
    pub cache_read_per_mtok_mc: Option<i64>,
    pub cache_write_per_mtok_mc: Option<i64>,
    /// The full, unmodified model object OpenRouter returned, so a field this module
    /// doesn't extract yet is never actually lost.
    pub raw: serde_json::Value,
}

/// Fetch every model OpenRouter reports, right now.
///
/// A single, well-formed but empty response is not an error — `parse_response` handles
/// that. A response that fails to parse as JSON at all, or whose top-level shape is not
/// `{"data": [...]}`, is: there is no sensible partial result to return.
pub async fn fetch(http: &reqwest::Client) -> Result<Vec<OpenRouterPricingRow>> {
    let body = http
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .map_err(|e| AegisError::Internal(format!("OpenRouter models request failed: {e}")))?
        .text()
        .await
        .map_err(|e| AegisError::Internal(format!("OpenRouter models response unreadable: {e}")))?;

    parse_response(&body)
}

/// Parse a raw response body into rows. Split out from [`fetch`] so it is testable
/// without a network call — every test in this module exercises this function against a
/// fixture string, never the live endpoint.
///
/// A model entry that fails to parse (an unexpected shape for one specific model) is
/// skipped with a warning rather than failing the whole batch — one odd model should not
/// cost visibility into the other few hundred.
pub fn parse_response(body: &str) -> Result<Vec<OpenRouterPricingRow>> {
    let response: ListResponse = serde_json::from_str(body).map_err(|e| {
        AegisError::Internal(format!(
            "OpenRouter response was not the expected {{\"data\": [...]}} shape: {e}"
        ))
    })?;

    let mut rows = Vec::with_capacity(response.data.len());
    for raw in response.data {
        match serde_json::from_value::<Model>(raw.clone()) {
            Ok(model) => rows.push(into_row(model, raw)),
            Err(e) => {
                let id = raw
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                tracing::warn!(model_id = id, error = %e, "skipping one OpenRouter model that did not parse");
            }
        }
    }
    Ok(rows)
}

fn into_row(model: Model, raw: serde_json::Value) -> OpenRouterPricingRow {
    OpenRouterPricingRow {
        input_per_mtok_mc: per_token_usd_to_micro_cents_per_mtok(
            &model.pricing.prompt,
            "prompt",
            &model.id,
        ),
        output_per_mtok_mc: per_token_usd_to_micro_cents_per_mtok(
            &model.pricing.completion,
            "completion",
            &model.id,
        ),
        cache_read_per_mtok_mc: per_token_usd_to_micro_cents_per_mtok(
            &model.pricing.input_cache_read,
            "input_cache_read",
            &model.id,
        ),
        cache_write_per_mtok_mc: per_token_usd_to_micro_cents_per_mtok(
            &model.pricing.input_cache_write,
            "input_cache_write",
            &model.id,
        ),
        context_length: model.context_length,
        display_name: model.name,
        model_id: model.id,
        raw,
    }
}

/// OpenRouter reports price as a decimal-string USD-per-token rate, e.g. `"0.0000025"`.
/// `model_pricing` (and this table, to keep the two directly comparable) stores
/// micro-cents per *million* tokens, so the conversion is: parse the string, scale by
/// 1,000,000 tokens to get USD-per-million, then apply the same
/// [`MicroCents::from_usd_per_mtok`] the seed table and the runbook both use.
///
/// A field OpenRouter omitted returns `None`. A field present but not parseable as a
/// number also returns `None` rather than panicking or silently storing a garbage
/// value — logged, so a persistently unparseable field is visible without crashing
/// ingestion of the other few hundred models in the same response.
fn per_token_usd_to_micro_cents_per_mtok(
    raw: &Option<String>,
    field: &str,
    model_id: &str,
) -> Option<i64> {
    let raw = raw.as_ref()?;
    match raw.parse::<f64>() {
        Ok(per_token_usd) if per_token_usd.is_finite() && per_token_usd >= 0.0 => {
            Some(MicroCents::from_usd_per_mtok(per_token_usd * 1_000_000.0).as_i64())
        }
        Ok(negative) => {
            tracing::warn!(
                model_id,
                field,
                value = negative,
                "OpenRouter reported a negative price; storing null for this field"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                model_id,
                field,
                raw,
                error = %e,
                "OpenRouter price was not a parseable number; storing null for this field"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic response shape: one fully-priced model, one with no cache pricing at
    /// all, one genuinely free model, and one with a malformed price — modeled on the
    /// real payload confirmed against the live endpoint while building this module.
    const FIXTURE: &str = r#"{
        "data": [
            {
                "id": "openai/gpt-4o",
                "name": "OpenAI: GPT-4o",
                "context_length": 128000,
                "pricing": {
                    "prompt": "0.0000025",
                    "completion": "0.00001",
                    "input_cache_read": "0.00000125"
                }
            },
            {
                "id": "mistralai/mistral-small",
                "name": "Mistral: Small",
                "context_length": 32000,
                "pricing": {
                    "prompt": "0.0000002",
                    "completion": "0.0000006"
                }
            },
            {
                "id": "meta-llama/llama-3-8b-free",
                "name": "Meta: Llama 3 8B (free)",
                "context_length": 8192,
                "pricing": {
                    "prompt": "0",
                    "completion": "0"
                }
            },
            {
                "id": "some-vendor/broken-pricing",
                "name": "A model with a malformed price",
                "context_length": 4096,
                "pricing": {
                    "prompt": "not-a-number",
                    "completion": "0.000001"
                }
            }
        ]
    }"#;

    #[test]
    fn a_fully_priced_model_converts_to_micro_cents_per_mtok() {
        let rows = parse_response(FIXTURE).unwrap();
        let gpt4o = rows.iter().find(|r| r.model_id == "openai/gpt-4o").unwrap();

        // $0.0000025/token * 1,000,000 tokens/Mtok = $2.50/Mtok = 2,500,000 micro-cents,
        // the exact worked example in docs/runbooks/pricing-update.md.
        assert_eq!(gpt4o.input_per_mtok_mc, Some(2_500_000));
        assert_eq!(gpt4o.output_per_mtok_mc, Some(10_000_000));
        assert_eq!(gpt4o.cache_read_per_mtok_mc, Some(1_250_000));
        assert_eq!(gpt4o.context_length, Some(128_000));
        assert_eq!(gpt4o.display_name, "OpenAI: GPT-4o");
    }

    #[test]
    fn an_absent_price_field_is_none_not_zero() {
        let rows = parse_response(FIXTURE).unwrap();
        let mistral = rows
            .iter()
            .find(|r| r.model_id == "mistralai/mistral-small")
            .unwrap();

        assert_eq!(
            mistral.cache_write_per_mtok_mc, None,
            "never reported, must be None"
        );
    }

    #[test]
    fn a_genuinely_free_model_is_some_zero_not_none() {
        let rows = parse_response(FIXTURE).unwrap();
        let free = rows
            .iter()
            .find(|r| r.model_id == "meta-llama/llama-3-8b-free")
            .unwrap();

        assert_eq!(
            free.input_per_mtok_mc,
            Some(0),
            "an explicit \"0\" must store as Some(0), not be conflated with 'not reported'"
        );
    }

    #[test]
    fn a_malformed_price_field_becomes_null_and_does_not_fail_the_batch() {
        let rows = parse_response(FIXTURE).unwrap();

        // All four models still made it through, including the one with a bad field.
        assert_eq!(rows.len(), 4);

        let broken = rows
            .iter()
            .find(|r| r.model_id == "some-vendor/broken-pricing")
            .unwrap();
        assert_eq!(
            broken.input_per_mtok_mc, None,
            "unparseable price -> null, not a guess"
        );
        // $0.000001/token * 1,000,000 tokens/Mtok = $1.00/Mtok = 1,000,000 micro-cents.
        assert_eq!(
            broken.output_per_mtok_mc,
            Some(1_000_000),
            "the other field on the same model is unaffected"
        );
    }

    #[test]
    fn the_raw_json_is_preserved_verbatim() {
        let rows = parse_response(FIXTURE).unwrap();
        let gpt4o = rows.iter().find(|r| r.model_id == "openai/gpt-4o").unwrap();
        assert_eq!(gpt4o.raw["id"], "openai/gpt-4o");
        assert_eq!(gpt4o.raw["pricing"]["prompt"], "0.0000025");
    }

    #[test]
    fn one_unparseable_model_does_not_take_down_the_whole_response() {
        let body = r#"{"data": [
            {"id": "fine/model", "name": "Fine", "pricing": {"prompt": "0.000001"}},
            {"name": "missing its id entirely, which this shape requires"}
        ]}"#;
        let rows = parse_response(body).unwrap();
        assert_eq!(rows.len(), 1, "the malformed entry is skipped, not fatal");
        assert_eq!(rows[0].model_id, "fine/model");
    }

    #[test]
    fn a_response_that_is_not_the_expected_shape_at_all_is_an_error() {
        let result = parse_response(r#"{"unexpected": "shape"}"#);
        assert!(
            result.is_err(),
            "no \"data\" array at all must fail loudly, not return empty"
        );
    }

    #[test]
    fn not_json_at_all_is_an_error_not_a_panic() {
        let result = parse_response("<html>this is not json</html>");
        assert!(result.is_err());
    }
}
