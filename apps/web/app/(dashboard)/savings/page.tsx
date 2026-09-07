"use client";

import { useEffect, useMemo, useState } from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { api, ApiError, type RequestLogRow, type UsageSummaryResponse } from "@/lib/api";
import {
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  Stat,
  UpgradeRequired,
} from "@/components/ui";
import {
  bareModelName,
  formatPercent,
  formatUsd,
  formatUsdCompact,
} from "@/lib/format";
import { useAuth } from "@/lib/auth-context";

/**
 * Tooltip value formatter.
 *
 * Recharts types a tooltip value as `ValueType | undefined`, which covers strings, arrays,
 * and missing data. Narrowing here rather than asserting a number keeps the chart from
 * rendering "NaN" when a series has a gap.
 */
function formatChartUsd(value: unknown): string {
  return typeof value === "number" ? `$${value.toFixed(4)}` : "—";
}

/**
 * The savings report.
 *
 * This page exists to be checked. A finance team should be able to export the CSV,
 * recompute every figure, and arrive at exactly our numbers — the arithmetic is integer
 * micro-cents specifically so that reconciliation is possible rather than approximate.
 *
 * The per-model breakdown is derived client-side from the request log rather than from a
 * separate aggregate endpoint, so what is charted is provably the same data the table
 * shows.
 */
export default function SavingsPage() {
  const { planFeatures } = useAuth();
  const hasSavings = planFeatures.savings === true;
  const [usage, setUsage] = useState<UsageSummaryResponse | null>(null);
  const [rows, setRows] = useState<RequestLogRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hasSavings) {
      setLoading(false);
      return;
    }
    let cancelled = false;

    Promise.all([api.usageSummary(), api.requests(500)])
      .then(([usageResponse, requestsResponse]) => {
        if (cancelled) return;
        setUsage(usageResponse);
        setRows(requestsResponse.requests);
        setLoading(false);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setError(
          caught instanceof ApiError ? caught.message : "Could not load savings data.",
        );
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [hasSavings]);

  /** Savings grouped by the model that actually served each request. */
  const byModel = useMemo(() => {
    const groups = new Map<
      string,
      { model: string; requests: number; spent: number; saved: number }
    >();

    for (const row of rows) {
      const key = row.served_model;
      const existing = groups.get(key) ?? {
        model: bareModelName(key),
        requests: 0,
        spent: 0,
        saved: 0,
      };
      existing.requests += 1;
      existing.spent += row.actual_cost_mc;
      existing.saved += row.gross_savings_mc;
      groups.set(key, existing);
    }

    return Array.from(groups.values())
      .sort((a, b) => b.saved - a.saved)
      .slice(0, 8)
      .map((group) => ({
        ...group,
        // Recharts wants plain numbers; convert micro-cents to dollars for the axis.
        spentUsd: group.spent / 1_000_000,
        savedUsd: group.saved / 1_000_000,
      }));
  }, [rows]);

  if (loading) {
    return <p className="text-sm text-[var(--color-muted-on-desk)]">Loading…</p>;
  }
  if (error) {
    return <ErrorState message={error} />;
  }
  if (!hasSavings) {
    return (
      <>
        <SectionHeader
          title="Savings"
          description="A live breakdown of what Aegis is saving you, by lever."
        />
        <UpgradeRequired feature="Savings & ROI" requiredPlan="Pro" />
      </>
    );
  }

  const summary = usage?.summary;
  const derived = usage?.derived;
  const hasData = (summary?.requests ?? 0) > 0;

  return (
    <>
      <SectionHeader
        title="Savings"
        description="Every figure here is the sum of per-request records. Export the CSV and the totals will reconcile to the micro-cent."
        action={
          <a
            href={api.savingsCsvUrl()}
            className="inline-block rounded-[var(--radius)] border border-[var(--color-line-dark)] px-3 py-1.5 text-sm text-[var(--color-muted)] transition-colors hover:bg-[var(--color-surface)] hover:text-[var(--color-ink)]"
          >
            Export CSV
          </a>
        }
      />

      {!hasData ? (
        <Card>
          <EmptyState
            title="Nothing to report yet"
            description="Once requests flow through the gateway, this page itemises exactly what each one saved and what we charged for it."
          />
        </Card>
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <Stat
              label="Would have cost"
              value={formatUsdCompact(summary?.baseline_cost_mc ?? 0)}
              sublabel="on the models you requested"
            />
            <Stat
              label="Actually cost"
              value={formatUsdCompact(summary?.actual_cost_mc ?? 0)}
              sublabel="paid to providers"
            />
            <Stat
              label="Gross savings"
              value={formatUsdCompact(summary?.gross_savings_mc ?? 0)}
              sublabel={formatPercent(derived?.savings_percent ?? 0)}
              accent
            />
            <Stat
              label="Your net saving"
              value={formatUsdCompact(derived?.customer_net_mc ?? 0)}
              sublabel={`after ${formatUsd(summary?.aegis_fee_mc ?? 0)} fee`}
              accent
            />
          </div>

          {derived && summary && summary.gross_savings_mc > 0 && (
            <Card className="mt-6 p-6">
              <h3 className="text-sm font-medium text-[var(--color-ink)]">
                Where the savings came from
              </h3>
              <p className="mt-1 text-xs text-[var(--color-muted-light)]">
                Gross savings split by lever. Compression and caching are each priced
                directly; routing absorbs whatever of the total those two don&rsquo;t
                explain, so the three always add up to the gross figure above exactly.
              </p>

              <div className="mt-5 grid gap-4 sm:grid-cols-3">
                <Stat
                  label="From routing"
                  value={formatUsdCompact(derived.savings_breakdown_mc.routing)}
                  sublabel="serving a cheaper capable model"
                />
                <Stat
                  label="From compression"
                  value={formatUsdCompact(derived.savings_breakdown_mc.compression)}
                  sublabel="tokens removed before the request was sent"
                />
                <Stat
                  label="From caching"
                  value={formatUsdCompact(derived.savings_breakdown_mc.cache)}
                  sublabel="exact/semantic hits, plus provider prompt-cache discounts"
                />
              </div>

              {(() => {
                const total = summary.gross_savings_mc || 1;
                const segments = [
                  {
                    label: "Routing",
                    mc: derived.savings_breakdown_mc.routing,
                    color: "var(--color-accent)",
                  },
                  {
                    label: "Compression",
                    mc: derived.savings_breakdown_mc.compression,
                    color: "var(--color-ochre)",
                  },
                  {
                    label: "Caching",
                    mc: derived.savings_breakdown_mc.cache,
                    color: "var(--color-positive)",
                  },
                ];
                return (
                  <div className="mt-5">
                    <div className="flex h-2.5 w-full overflow-hidden rounded-full border border-[var(--color-ink)]">
                      {segments.map((segment) => (
                        <div
                          key={segment.label}
                          style={{
                            width: `${Math.max(0, (segment.mc / total) * 100)}%`,
                            backgroundColor: segment.color,
                          }}
                          title={`${segment.label}: ${formatUsd(segment.mc)}`}
                        />
                      ))}
                    </div>
                    <div className="mt-2 flex flex-wrap gap-x-5 gap-y-1">
                      {segments.map((segment) => (
                        <span
                          key={segment.label}
                          className="flex items-center gap-1.5 text-[11px] font-medium text-[var(--color-muted)]"
                        >
                          <span
                            className="h-2 w-2 rounded-full"
                            style={{ backgroundColor: segment.color }}
                          />
                          {segment.label} — {formatPercent((segment.mc / total) * 100)}
                        </span>
                      ))}
                    </div>
                  </div>
                );
              })()}
            </Card>
          )}

          {byModel.length > 0 && (
            <Card className="mt-6 p-6">
              <h3 className="text-sm font-medium text-[var(--color-ink)]">
                Savings by served model
              </h3>
              <p className="mt-1 text-xs text-[var(--color-muted-light)]">
                What each model cost, against what it saved versus the model requested.
              </p>

              <div className="mt-6 h-72">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart
                    data={byModel}
                    margin={{ top: 4, right: 8, bottom: 4, left: 8 }}
                  >
                    <CartesianGrid
                      strokeDasharray="3 3"
                      stroke="var(--color-line)"
                      vertical={false}
                    />
                    <XAxis
                      dataKey="model"
                      stroke="var(--color-muted-light)"
                      fontSize={11}
                      tickLine={false}
                      axisLine={{ stroke: "var(--color-line)" }}
                    />
                    <YAxis
                      stroke="var(--color-muted-light)"
                      fontSize={11}
                      tickLine={false}
                      axisLine={false}
                      tickFormatter={(value: number) => `$${Number(value).toFixed(2)}`}
                    />
                    <Tooltip
                      cursor={{ fill: "var(--color-surface)" }}
                      contentStyle={{
                        backgroundColor: "var(--color-surface2)",
                        border: "1px solid var(--color-line-dark)",
                        borderRadius: "6px",
                        fontSize: "12px",
                      }}
                      labelStyle={{ color: "var(--color-ink)" }}
                      formatter={(value, name) => [formatChartUsd(value), name]}
                    />
                    <Legend
                      wrapperStyle={{ fontSize: "12px" }}
                      iconType="square"
                      iconSize={8}
                    />
                    <Bar
                      dataKey="spentUsd"
                      name="Spent"
                      fill="var(--color-muted-light)"
                      radius={[2, 2, 0, 0]}
                    />
                    <Bar
                      dataKey="savedUsd"
                      name="Saved"
                      fill="var(--color-accent)"
                      radius={[2, 2, 0, 0]}
                    />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            </Card>
          )}

          <Card className="mt-6 p-6">
            <h3 className="text-sm font-medium text-[var(--color-ink)]">
              How the fee is calculated
            </h3>
            <p className="mt-2 text-sm leading-relaxed text-[var(--color-muted)]">
              We charge a share of savings we actually delivered, and only when the figure
              is positive. If a routing decision costs more than the model you asked for
              would have, the saving floors at zero and no fee applies — that overspend is
              ours to absorb.
            </p>
            <pre className="tabular mt-4 overflow-x-auto rounded-[var(--radius)] border border-[#3C3324] bg-[#201B14] p-4 text-xs text-[#B8AC8E]">
              <code>{`baseline_cost  = ${formatUsd(summary?.baseline_cost_mc ?? 0)}
actual_cost    = ${formatUsd(summary?.actual_cost_mc ?? 0)}
gross_savings  = ${formatUsd(summary?.gross_savings_mc ?? 0)}
aegis_fee      = ${formatUsd(summary?.aegis_fee_mc ?? 0)}
customer_net   = ${formatUsd(derived?.customer_net_mc ?? 0)}`}</code>
            </pre>
          </Card>
        </>
      )}
    </>
  );
}
