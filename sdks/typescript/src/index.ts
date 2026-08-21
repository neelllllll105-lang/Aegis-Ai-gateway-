/**
 * Aegis TypeScript SDK.
 *
 * # Why this is thin on purpose
 *
 * The integration story is "change one base URL". An SDK that wraps chat completions in
 * its own types would undercut that: it becomes a thing to learn, a thing to migrate to,
 * and a thing to migrate away from — which is exactly the lock-in we tell customers we do
 * not create.
 *
 * So this package deliberately does not wrap the chat API. Keep using the OpenAI or
 * Anthropic SDK you already have, pointed at Aegis. What this adds is the part those SDKs
 * cannot give you: typed access to the savings attribution on every response, and the
 * management API.
 *
 * @packageDocumentation
 */

/** Default gateway URL. */
export const DEFAULT_BASE_URL = "https://api.aegis.dev";

/** Micro-cents in one US dollar. All monetary values cross the API as integers. */
export const MICRO_CENTS_PER_USD = 1_000_000;

/**
 * The savings attribution Aegis reports on every response.
 *
 * Parsed from `X-Aegis-*` headers. All amounts are micro-cents.
 */
export interface AegisAttribution {
  /** The model that actually served the request. */
  servedModel: string;
  /** The model the caller asked for — the savings baseline. */
  requestedModel: string;
  /** What this request cost, in micro-cents. Zero on a cache hit. */
  costMicroCents: number;
  /** What the requested model would have cost, in micro-cents. */
  baselineCostMicroCents: number;
  /** The difference, in micro-cents. */
  savingsMicroCents: number;
  /** `exact`, `semantic`, `miss`, or `skipped`. */
  cache: string;
  /** Why this model was chosen. */
  routing: string;
  /** Aegis's own added latency in milliseconds, excluding the provider call. */
  overheadMs: number | null;
  /** Total request latency in milliseconds. */
  latencyMs: number | null;
  requestId: string;
}

/** Anything with a `get`-style header accessor: `fetch` Headers, axios, node-fetch. */
export interface HeaderSource {
  get(name: string): string | null | undefined;
}

/**
 * Extract the savings attribution from a response.
 *
 * Accepts a `Response`, a `Headers`, or any plain object of headers, so it works with
 * whichever HTTP client the caller already uses.
 *
 * Returns `null` when the headers are absent — which means the response did not come
 * through Aegis, and is worth distinguishing from a request that saved nothing.
 *
 * @example
 * ```ts
 * const response = await fetch(`${base}/v1/chat/completions`, { ... });
 * const attribution = parseAttribution(response);
 * if (attribution) {
 *   console.log(`saved ${formatUsd(attribution.savingsMicroCents)}`);
 * }
 * ```
 */
export function parseAttribution(
  source: HeaderSource | Headers | Response | Record<string, string>,
): AegisAttribution | null {
  const get = buildHeaderReader(source);

  const servedModel = get("x-aegis-model");
  const requestId = get("x-aegis-request-id");

  // Both are present on every Aegis response. Their absence means this response came
  // from somewhere else.
  if (!servedModel || !requestId) return null;

  const latency = get("x-aegis-latency");
  const parsed = latency ? parseLatency(latency) : { total: null, overhead: null };

  return {
    servedModel,
    requestedModel: get("x-aegis-requested-model") ?? servedModel,
    costMicroCents: parseUsdHeader(get("x-aegis-cost")),
    baselineCostMicroCents: parseUsdHeader(get("x-aegis-baseline-cost")),
    savingsMicroCents: parseUsdHeader(get("x-aegis-savings")),
    cache: get("x-aegis-cache") ?? "unknown",
    routing: get("x-aegis-routing") ?? "unknown",
    overheadMs: parsed.overhead,
    latencyMs: parsed.total,
    requestId,
  };
}

function buildHeaderReader(
  source: HeaderSource | Headers | Response | Record<string, string>,
): (name: string) => string | undefined {
  // A Response carries its headers on `.headers`.
  if (typeof source === "object" && source !== null && "headers" in source) {
    const headers = (source as Response).headers;
    if (headers && typeof headers.get === "function") {
      return (name) => headers.get(name) ?? undefined;
    }
  }

  if (typeof (source as HeaderSource).get === "function") {
    return (name) => (source as HeaderSource).get(name) ?? undefined;
  }

  // A plain object. Header names are case-insensitive, so normalise both sides.
  const lowered: Record<string, string> = {};
  for (const [key, value] of Object.entries(source as Record<string, string>)) {
    lowered[key.toLowerCase()] = value;
  }
  return (name) => lowered[name.toLowerCase()];
}

/**
 * Parse a `$0.007050` header into micro-cents.
 *
 * Returns an integer, so arithmetic on the result stays exact — the same reason the
 * gateway uses integers internally.
 */
function parseUsdHeader(value: string | undefined): number {
  if (!value) return 0;
  const numeric = Number.parseFloat(value.replace(/[$,]/g, ""));
  if (!Number.isFinite(numeric)) return 0;
  return Math.round(numeric * MICRO_CENTS_PER_USD);
}

/** Parse `842ms (overhead: 0.371ms)`. */
function parseLatency(value: string): { total: number | null; overhead: number | null } {
  const total = value.match(/^([\d.]+)ms/);
  const overhead = value.match(/overhead:\s*([\d.]+)ms/);
  return {
    total: total?.[1] ? Number.parseFloat(total[1]) : null,
    overhead: overhead?.[1] ? Number.parseFloat(overhead[1]) : null,
  };
}

/** Format micro-cents as a USD string, with precision that suits the magnitude. */
export function formatUsd(microCents: number): string {
  const usd = microCents / MICRO_CENTS_PER_USD;
  if (usd === 0) return "$0.00";
  if (Math.abs(usd) < 0.01) return `$${usd.toFixed(6)}`;
  if (Math.abs(usd) < 1) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

/** Routing hints accepted by the `X-Aegis-Routing-Hint` header. */
export type RoutingHint = "auto" | "passthrough" | "cheap";

/**
 * Headers that force a routing decision.
 *
 * `passthrough` guarantees the model you asked for. That escape hatch is permanent.
 *
 * @example
 * ```ts
 * await client.chat.completions.create(
 *   { model: "gpt-4o", messages },
 *   { headers: routingHeaders("passthrough") },
 * );
 * ```
 */
export function routingHeaders(hint: RoutingHint): Record<string, string> {
  return { "X-Aegis-Routing-Hint": hint };
}

/** Options for {@link AegisClient}. */
export interface AegisClientOptions {
  apiKey: string;
  baseUrl?: string;
  fetch?: typeof fetch;
}

/** A usage summary from the management API. */
export interface UsageSummary {
  requests: number;
  cacheHits: number;
  inputTokens: number;
  outputTokens: number;
  baselineCostMicroCents: number;
  actualCostMicroCents: number;
  grossSavingsMicroCents: number;
  aegisFeeMicroCents: number;
  /** What you keep after our share. */
  customerNetMicroCents: number;
  savingsPercent: number;
  cacheHitRate: number;
}

/**
 * A minimal client for the management API.
 *
 * For reading your own usage and savings programmatically — building an internal
 * dashboard, or exporting to a finance system. It does not wrap chat completions; use
 * your existing SDK for that.
 */
export class AegisClient {
  private readonly apiKey: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: AegisClientOptions) {
    if (!options.apiKey) {
      throw new Error("AegisClient requires an apiKey");
    }
    this.apiKey = options.apiKey;
    this.baseUrl = (options.baseUrl ?? DEFAULT_BASE_URL).replace(/\/$/, "");
    this.fetchImpl = options.fetch ?? globalThis.fetch;
  }

  /** Usage and savings for a period, defaulting to the last 30 days. */
  async usageSummary(start?: Date, end?: Date): Promise<UsageSummary> {
    const params = new URLSearchParams();
    if (start) params.set("start", start.toISOString());
    if (end) params.set("end", end.toISOString());

    const query = params.toString();
    const body = await this.request<{
      summary: {
        requests: number;
        cache_hits: number;
        input_tokens: number;
        output_tokens: number;
        baseline_cost_mc: number;
        actual_cost_mc: number;
        gross_savings_mc: number;
        aegis_fee_mc: number;
      };
      derived: {
        savings_percent: number;
        cache_hit_rate: number;
        customer_net_mc: number;
      };
    }>(`/api/usage/summary${query ? `?${query}` : ""}`);

    return {
      requests: body.summary.requests,
      cacheHits: body.summary.cache_hits,
      inputTokens: body.summary.input_tokens,
      outputTokens: body.summary.output_tokens,
      baselineCostMicroCents: body.summary.baseline_cost_mc,
      actualCostMicroCents: body.summary.actual_cost_mc,
      grossSavingsMicroCents: body.summary.gross_savings_mc,
      aegisFeeMicroCents: body.summary.aegis_fee_mc,
      customerNetMicroCents: body.derived.customer_net_mc,
      savingsPercent: body.derived.savings_percent,
      cacheHitRate: body.derived.cache_hit_rate,
    };
  }

  /** Models available to your organisation, with their prices. */
  async models(): Promise<unknown[]> {
    const body = await this.request<{ data: unknown[] }>("/v1/models");
    return body.data;
  }

  private async request<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      headers: {
        Authorization: `Bearer ${this.apiKey}`,
        "Content-Type": "application/json",
      },
    });

    if (!response.ok) {
      // Surface the gateway's machine-readable error type rather than a bare status.
      let message = `Aegis request failed with status ${response.status}`;
      try {
        const body = (await response.json()) as {
          error?: { type?: string; message?: string };
        };
        if (body.error?.message) {
          message = `${body.error.type ?? "error"}: ${body.error.message}`;
        }
      } catch {
        // Leave the status-based default.
      }
      throw new Error(message);
    }

    return (await response.json()) as T;
  }
}
