"use client";

import { useEffect, useMemo, useState } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { api, ApiError, type MyUsageResponse, type RequestLogRow } from "@/lib/api";
import { Card, EmptyState, ErrorState, SectionHeader, Stat } from "@/components/ui";
import { formatCount, formatPercent, formatUsd, formatUsdCompact } from "@/lib/format";

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
 * Usage over time — the "savings ribbon".
 *
 * The chart plots what each day *would* have cost against what it actually did. The gap
 * between the two lines is the product, which is why they share an axis rather than being
 * shown as separate charts: the comparison is the point.
 */
export default function UsagePage() {
  const [rows, setRows] = useState<RequestLogRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Separate from the org-wide log above: what's attributed to the signed-in person
  // specifically, across every key issued to them. Loaded independently and allowed to
  // fail quietly — a service-account-only session has no "me" to report on, and that must
  // not block the rest of the page.
  const [myUsage, setMyUsage] = useState<MyUsageResponse | null>(null);

  useEffect(() => {
    let cancelled = false;

    api
      .requests(1000)
      .then((response) => {
        if (cancelled) return;
        setRows(response.requests);
        setLoading(false);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setError(
          caught instanceof ApiError ? caught.message : "Could not load usage data.",
        );
        setLoading(false);
      });

    api
      .myUsage()
      .then((response) => {
        if (!cancelled) setMyUsage(response);
      })
      .catch(() => {
        // No "me" to report on for this credential — the org-wide view above still holds.
      });

    return () => {
      cancelled = true;
    };
  }, []);

  /** Daily totals, oldest first. */
  const daily = useMemo(() => {
    const days = new Map<
      string,
      { day: string; baseline: number; actual: number; requests: number }
    >();

    for (const row of rows) {
      const day = row.created_at.slice(0, 10);
      const existing = days.get(day) ?? {
        day,
        baseline: 0,
        actual: 0,
        requests: 0,
      };
      existing.baseline += row.baseline_cost_mc;
      existing.actual += row.actual_cost_mc;
      existing.requests += 1;
      days.set(day, existing);
    }

    return Array.from(days.values())
      .sort((a, b) => a.day.localeCompare(b.day))
      .map((entry) => ({
        ...entry,
        label: entry.day.slice(5),
        baselineUsd: entry.baseline / 1_000_000,
        actualUsd: entry.actual / 1_000_000,
      }));
  }, [rows]);

  const totals = useMemo(
    () =>
      rows.reduce(
        (accumulator, row) => ({
          requests: accumulator.requests + 1,
          baseline: accumulator.baseline + row.baseline_cost_mc,
          actual: accumulator.actual + row.actual_cost_mc,
        }),
        { requests: 0, baseline: 0, actual: 0 },
      ),
    [rows],
  );

  if (loading) {
    return <p className="text-sm text-[var(--color-muted-on-desk)]">Loading…</p>;
  }
  if (error) {
    return <ErrorState message={error} />;
  }

  return (
    <>
      <SectionHeader
        title="Usage"
        description="Daily spend against what the same traffic would have cost unoptimised. The gap is what Aegis removed."
      />

      {myUsage && myUsage.summary.requests > 0 && (
        <Card className="mb-6 p-6">
          <h3 className="text-sm font-medium text-[var(--color-ink)]">
            Your own usage
          </h3>
          <p className="mt-1 text-xs text-[var(--color-muted-light)]">
            Attributed to keys issued to you specifically, last 30 days — a subset of the
            organisation-wide totals below.
          </p>
          <div className="mt-4 grid gap-4 sm:grid-cols-3">
            <Stat
              label="You spent"
              value={formatUsdCompact(myUsage.summary.actual_cost_mc)}
              sublabel={`${formatCount(myUsage.summary.requests)} requests`}
            />
            <Stat
              label="You saved"
              value={formatUsdCompact(myUsage.summary.gross_savings_mc)}
              sublabel={formatPercent(myUsage.derived.savings_percent)}
              accent
            />
            <Stat
              label="Cache hit rate"
              value={formatPercent(myUsage.derived.cache_hit_rate)}
            />
          </div>
        </Card>
      )}

      {daily.length === 0 ? (
        <Card>
          <EmptyState
            title="No usage yet"
            description="Send a request through the gateway and this chart fills in from the first day."
          />
        </Card>
      ) : (
        <>
          <Card className="p-6">
            <div className="flex flex-wrap items-baseline justify-between gap-4">
              <h3 className="text-sm font-medium text-[var(--color-ink)]">
                Spend vs baseline
              </h3>
              <div className="text-xs text-[var(--color-muted-light)]">
                <span className="tabular text-[var(--color-muted)]">
                  {formatCount(totals.requests)}
                </span>{" "}
                requests ·{" "}
                <span className="tabular text-[var(--color-accent)]">
                  {formatUsd(totals.baseline - totals.actual)}
                </span>{" "}
                saved
              </div>
            </div>

            <div className="mt-6 h-80">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart
                  data={daily}
                  margin={{ top: 4, right: 8, bottom: 4, left: 8 }}
                >
                  <defs>
                    <linearGradient id="baselineFill" x1="0" y1="0" x2="0" y2="1">
                      <stop
                        offset="0%"
                        stopColor="var(--color-muted-light)"
                        stopOpacity={0.25}
                      />
                      <stop
                        offset="100%"
                        stopColor="var(--color-muted-light)"
                        stopOpacity={0.02}
                      />
                    </linearGradient>
                    <linearGradient id="actualFill" x1="0" y1="0" x2="0" y2="1">
                      <stop
                        offset="0%"
                        stopColor="var(--color-accent)"
                        stopOpacity={0.3}
                      />
                      <stop
                        offset="100%"
                        stopColor="var(--color-accent)"
                        stopOpacity={0.02}
                      />
                    </linearGradient>
                  </defs>

                  <CartesianGrid
                    strokeDasharray="3 3"
                    stroke="var(--color-line)"
                    vertical={false}
                  />
                  <XAxis
                    dataKey="label"
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
                    iconType="line"
                    iconSize={12}
                  />
                  <Area
                    type="monotone"
                    dataKey="baselineUsd"
                    name="Would have cost"
                    stroke="var(--color-muted-light)"
                    strokeDasharray="4 3"
                    fill="url(#baselineFill)"
                    strokeWidth={1.5}
                  />
                  <Area
                    type="monotone"
                    dataKey="actualUsd"
                    name="Actually cost"
                    stroke="var(--color-accent)"
                    fill="url(#actualFill)"
                    strokeWidth={2}
                  />
                </AreaChart>
              </ResponsiveContainer>
            </div>
          </Card>

          <Card className="mt-6 p-6">
            <h3 className="text-sm font-medium text-[var(--color-ink)]">Daily breakdown</h3>
            <div className="mt-4 overflow-x-auto">
              <table className="w-full min-w-[420px] text-sm">
                <thead>
                  <tr>
                    <th className="hairline px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-[var(--color-muted-light)]">
                      Day
                    </th>
                    <th className="hairline px-3 py-2 text-right text-xs font-medium uppercase tracking-wide text-[var(--color-muted-light)]">
                      Requests
                    </th>
                    <th className="hairline px-3 py-2 text-right text-xs font-medium uppercase tracking-wide text-[var(--color-muted-light)]">
                      Baseline
                    </th>
                    <th className="hairline px-3 py-2 text-right text-xs font-medium uppercase tracking-wide text-[var(--color-muted-light)]">
                      Actual
                    </th>
                    <th className="hairline px-3 py-2 text-right text-xs font-medium uppercase tracking-wide text-[var(--color-muted-light)]">
                      Saved
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {[...daily].reverse().map((entry) => (
                    <tr key={entry.day}>
                      <td className="hairline px-3 py-2 text-[var(--color-muted)]">
                        {entry.day}
                      </td>
                      <td className="tabular hairline px-3 py-2 text-right text-[var(--color-muted)]">
                        {formatCount(entry.requests)}
                      </td>
                      <td className="tabular hairline px-3 py-2 text-right text-[var(--color-muted-light)]">
                        {formatUsd(entry.baseline)}
                      </td>
                      <td className="tabular hairline px-3 py-2 text-right text-[var(--color-muted)]">
                        {formatUsd(entry.actual)}
                      </td>
                      <td className="tabular hairline px-3 py-2 text-right text-[var(--color-accent)]">
                        {formatUsd(entry.baseline - entry.actual)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>
        </>
      )}
    </>
  );
}
