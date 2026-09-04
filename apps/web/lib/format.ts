/**
 * Formatting helpers.
 *
 * Every monetary value crosses the API as an integer count of micro-cents
 * (1 cent = 10,000; 1 USD = 1,000,000). See `docs/adr/0002-money-as-integers.md`.
 *
 * The conversion to a human-readable string happens here and only here. The rule that
 * matters: a number the customer might reconcile against their own records must never be
 * rounded more aggressively than the amount deserves. Displaying a $0.0074 saving as
 * "$0.01" makes our arithmetic look wrong when they check it.
 */

/** Micro-cents in one US dollar. */
export const MICRO_CENTS_PER_USD = 1_000_000;

/**
 * Format micro-cents as USD.
 *
 * Precision adapts to magnitude, because these figures span seven orders of magnitude:
 * a single request costs fractions of a cent, and a monthly invoice is hundreds of
 * dollars. A fixed two decimal places would render most per-request costs as "$0.00".
 */
export function formatUsd(microCents: number | null | undefined): string {
  if (microCents === null || microCents === undefined || Number.isNaN(microCents)) {
    return "$0.00";
  }

  const usd = microCents / MICRO_CENTS_PER_USD;
  const magnitude = Math.abs(usd);

  if (magnitude === 0) return "$0.00";
  // Sub-cent: show enough digits that the value is not simply "zero".
  if (magnitude < 0.01) return `$${usd.toFixed(6)}`;
  if (magnitude < 1) return `$${usd.toFixed(4)}`;
  if (magnitude < 10_000) {
    return `$${usd.toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })}`;
  }
  return `$${usd.toLocaleString("en-US", { maximumFractionDigits: 0 })}`;
}

/**
 * Format micro-cents compactly for a headline figure: `$1.2k`, `$340`.
 *
 * For dashboard tiles only. Never use this where a customer might reconcile the number.
 */
export function formatUsdCompact(microCents: number | null | undefined): string {
  if (!microCents) return "$0";
  const usd = microCents / MICRO_CENTS_PER_USD;
  const magnitude = Math.abs(usd);

  if (magnitude >= 1_000_000) return `$${(usd / 1_000_000).toFixed(1)}M`;
  if (magnitude >= 1_000) return `$${(usd / 1_000).toFixed(1)}k`;
  if (magnitude >= 1) return `$${usd.toFixed(2)}`;
  return formatUsd(microCents);
}

/** Format a whole number with thousands separators. */
export function formatCount(value: number | null | undefined): string {
  if (!value) return "0";
  return value.toLocaleString("en-US");
}

/** Format a token count compactly: `1.2M`, `340k`. */
export function formatTokens(value: number | null | undefined): string {
  if (!value) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toString();
}

/** Format a percentage to one decimal place. */
export function formatPercent(value: number | null | undefined): string {
  if (value === null || value === undefined || Number.isNaN(value)) return "0.0%";
  return `${value.toFixed(1)}%`;
}

/** Format a duration in milliseconds. */
export function formatLatency(ms: number | null | undefined): string {
  if (!ms) return "0ms";
  if (ms < 1) return `${(ms * 1000).toFixed(0)}µs`;
  if (ms < 1000) return `${ms.toFixed(0)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/** Format an ISO timestamp as a short absolute date and time. */
export function formatTimestamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/**
 * Format an ISO timestamp as a relative age.
 *
 * Falls back to an absolute date beyond a week: "43 days ago" is harder to act on than
 * a date, and precision stops being useful at that range.
 */
export function formatRelative(iso: string | null | undefined): string {
  if (!iso) return "never";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "unknown";

  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  if (seconds < 0) return "just now";
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h ago`;
  if (seconds < 604_800) return `${Math.floor(seconds / 86_400)}d ago`;

  return date.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

/** Truncate a model id to its bare name: `openai/gpt-4o` becomes `gpt-4o`. */
export function bareModelName(modelId: string): string {
  const slash = modelId.indexOf("/");
  return slash === -1 ? modelId : modelId.slice(slash + 1);
}

/**
 * The savings-share rate for a plan, as a percentage.
 *
 * Mirrors `money::savings_share_basis_points` in the gateway. Duplicated deliberately —
 * the pricing calculator must work before a visitor has an account, so it cannot ask the
 * API. Any change to the business model must update both.
 */
export function savingsSharePercent(plan: string): number {
  switch (plan) {
    case "pro":
      return 20;
    case "team":
      return 15;
    case "enterprise":
      return 10;
    default:
      return 0;
  }
}
