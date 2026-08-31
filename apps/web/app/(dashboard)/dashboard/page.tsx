"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  type OrgResponse,
  type UsageSummaryResponse,
} from "@/lib/api";
import {
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  Stat,
} from "@/components/ui";
import {
  formatCount,
  formatPercent,
  formatTokens,
  formatUsd,
  formatUsdCompact,
} from "@/lib/format";

/**
 * The overview.
 *
 * `MASTER_BUILD.md` calls this "the money page". The single number that matters is what
 * the customer *kept* — not gross savings, which flatters us by including our own fee.
 * Showing the gross figure as the headline would be the small dishonesty that makes
 * everything else on the page suspect.
 */
export default function DashboardPage() {
  const [org, setOrg] = useState<OrgResponse | null>(null);
  const [usage, setUsage] = useState<UsageSummaryResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    Promise.all([api.org(), api.usageSummary()])
      .then(([orgResponse, usageResponse]) => {
        if (cancelled) return;
        setOrg(orgResponse);
        setUsage(usageResponse);
        setLoading(false);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setError(
          caught instanceof ApiError ? caught.message : "Could not load your usage.",
        );
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (loading) {
    return <p className="text-sm text-[var(--color-muted-on-desk)]">Loading…</p>;
  }

  if (error) {
    return <ErrorState message={error} />;
  }

  const summary = usage?.summary;
  const derived = usage?.derived;
  const hasTraffic = (summary?.requests ?? 0) > 0;

  return (
    <>
      <SectionHeader
        title="Overview"
        description={
          org
            ? `${org.organization.name} · ${org.organization.plan} plan · last 30 days`
            : undefined
        }
      />

      {!hasTraffic ? (
        <Card>
          <EmptyState
            title="No requests yet"
            description="Point a client at the gateway and your first request will appear here within seconds, along with what it cost and what it saved."
            action={
              <Link
                href="/keys"
                className="inline-block rounded-[var(--radius)] bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-[var(--color-surface)]"
              >
                Create an API key
              </Link>
            }
          />
        </Card>
      ) : (
        <>
          {/* The headline: what the customer kept, after our fee. */}
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <Stat
              label="You kept"
              value={formatUsdCompact(derived?.customer_net_mc ?? 0)}
              sublabel={`after our ${formatUsd(summary?.aegis_fee_mc ?? 0)} share`}
              accent
            />
            <Stat
              label="You spent"
              value={formatUsdCompact(summary?.actual_cost_mc ?? 0)}
              sublabel={`vs ${formatUsd(summary?.baseline_cost_mc ?? 0)} unoptimised`}
            />
            <Stat
              label="Requests"
              value={formatCount(summary?.requests ?? 0)}
              sublabel={`${formatPercent(derived?.cache_hit_rate ?? 0)} served from cache`}
            />
            <Stat
              label="Bill reduction"
              value={formatPercent(derived?.savings_percent ?? 0)}
              sublabel="of what you would have paid"
            />
          </div>

          {/* The full arithmetic, unaggregated. Anyone can check it. */}
          <Card className="mt-6 p-6">
            <h3 className="text-sm font-medium text-[var(--color-ink)]">
              Where the money went
            </h3>
            <p className="mt-1 text-xs text-[var(--color-muted-light)]">
              Every figure below is the sum of per-request records. Export the itemised CSV
              from the Savings page and the totals will match exactly.
            </p>

            <dl className="mt-5 space-y-3">
              <LedgerRow
                label="What these requests would have cost on the models you asked for"
                value={formatUsd(summary?.baseline_cost_mc ?? 0)}
              />
              <LedgerRow
                label="What we actually paid providers"
                value={`− ${formatUsd(summary?.actual_cost_mc ?? 0)}`}
              />
              <div className="border-t border-[var(--color-line)] pt-3">
                <LedgerRow
                  label="Gross savings"
                  value={formatUsd(summary?.gross_savings_mc ?? 0)}
                  emphasis
                />
              </div>
              <LedgerRow
                label={`Aegis savings share (${((org?.organization.savings_share_bp ?? 0) / 100).toFixed(0)}%)`}
                value={`− ${formatUsd(summary?.aegis_fee_mc ?? 0)}`}
              />
              <div className="border-t border-[var(--color-line)] pt-3">
                <LedgerRow
                  label="Your net saving"
                  value={formatUsd(derived?.customer_net_mc ?? 0)}
                  emphasis
                  accent
                />
              </div>
            </dl>
          </Card>

          <div className="mt-6 grid gap-4 sm:grid-cols-3">
            <Card className="p-5">
              <div className="text-xs uppercase tracking-wide text-[var(--color-muted-light)]">
                Tokens processed
              </div>
              <div className="tabular mt-2 text-xl text-[var(--color-ink)]">
                {formatTokens(
                  (summary?.input_tokens ?? 0) + (summary?.output_tokens ?? 0),
                )}
              </div>
              <div className="mt-1 text-xs text-[var(--color-muted-light)]">
                {formatTokens(summary?.input_tokens ?? 0)} in ·{" "}
                {formatTokens(summary?.output_tokens ?? 0)} out
              </div>
            </Card>

            <Card className="p-5">
              <div className="text-xs uppercase tracking-wide text-[var(--color-muted-light)]">
                Cache hits
              </div>
              <div className="tabular mt-2 text-xl text-[var(--color-ink)]">
                {formatCount(summary?.cache_hits ?? 0)}
              </div>
              <div className="mt-1 text-xs text-[var(--color-muted-light)]">
                served without an upstream call
              </div>
            </Card>

            <Card className="p-5">
              <div className="text-xs uppercase tracking-wide text-[var(--color-muted-light)]">
                This month
              </div>
              <div className="tabular mt-2 text-xl text-[var(--color-ink)]">
                {formatCount(org?.usage.month_to_date_requests ?? 0)}
              </div>
              <div className="mt-1 text-xs text-[var(--color-muted-light)]">
                requests month to date
              </div>
            </Card>
          </div>
        </>
      )}
    </>
  );
}

function LedgerRow({
  label,
  value,
  emphasis = false,
  accent = false,
}: {
  label: string;
  value: string;
  emphasis?: boolean;
  accent?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-6">
      <dt
        className={`text-sm ${
          emphasis ? "text-[var(--color-ink)]" : "text-[var(--color-muted-light)]"
        }`}
      >
        {label}
      </dt>
      <dd
        className={`tabular shrink-0 ${
          accent
            ? "text-[var(--color-accent)]"
            : emphasis
              ? "text-[var(--color-ink)]"
              : "text-[var(--color-muted)]"
        }`}
      >
        {value}
      </dd>
    </div>
  );
}
