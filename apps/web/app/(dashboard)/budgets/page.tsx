"use client";

import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  type AnomalyReport,
  type Budget,
  type Team,
} from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  DisclosureMeter,
  EmptyState,
  ErrorState,
  SectionHeader,
  TableShell,
  Td,
  Th,
} from "@/components/ui";
import { formatUsd } from "@/lib/format";

const PERIODS = ["daily", "weekly", "monthly"] as const;

/**
 * Budgets and spend anomaly detection.
 *
 * These are two different safety nets and the page keeps them visually distinct because
 * they catch different failures. A budget catches spend above a line you chose. An
 * anomaly catches spend that is *within* the line but nothing like normal — the runaway
 * agent loop that burns a month of budget in an afternoon and is invisible until the
 * invoice arrives.
 */
export default function BudgetsPage() {
  const [budgets, setBudgets] = useState<Budget[]>([]);
  const [teams, setTeams] = useState<Team[]>([]);
  const [anomaly, setAnomaly] = useState<AnomalyReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const [scope, setScope] = useState<string>("org");
  const [period, setPeriod] = useState<string>("monthly");
  const [limitUsd, setLimitUsd] = useState("500");
  const [hardLimit, setHardLimit] = useState(true);

  async function load() {
    try {
      // The anomaly report is advisory: if it fails, the budgets still matter and the
      // page should still render. So it is settled separately rather than rejecting the
      // whole load.
      const [budgetResponse, teamResponse] = await Promise.all([
        api.listBudgets(),
        api.listTeams(),
      ]);
      setBudgets(budgetResponse.budgets);
      setTeams(teamResponse.teams);
      setError(null);

      try {
        setAnomaly(await api.anomalies());
      } catch {
        setAnomaly(null);
      }
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not load budgets.",
      );
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleCreate(event: React.FormEvent) {
    event.preventDefault();

    const dollars = Number.parseFloat(limitUsd);
    if (!Number.isFinite(dollars) || dollars <= 0) {
      setError("Enter a budget limit greater than zero.");
      return;
    }

    setSaving(true);
    try {
      await api.createBudget({
        team_id: scope === "org" ? null : scope,
        period,
        // Dollars to micro-cents. Rounded once, here, so the integer that reaches the
        // server is exact — the server never sees a float.
        limit_mc: Math.round(dollars * 1_000_000),
        hard_limit: hardLimit,
      });
      setLimitUsd("500");
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not create the budget.",
      );
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(budget: Budget) {
    const confirmed = window.confirm(
      `Remove this ${budget.period} budget of ${formatUsd(budget.limit_mc)}? Spend will no longer be capped.`,
    );
    if (!confirmed) return;

    try {
      await api.deleteBudget(budget.id);
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not remove the budget.",
      );
    }
  }

  function scopeLabel(budget: Budget): string {
    if (budget.api_key_id) return "Single API key";
    if (!budget.team_id) return "Whole organisation";
    return teams.find((team) => team.id === budget.team_id)?.name ?? "Team";
  }

  return (
    <>
      <SectionHeader
        eyebrow="Governance"
        title="Budgets and spend alerts"
        description="A hard budget rejects requests once the limit is reached. A soft budget lets them through and records the breach. Both are evaluated before the request reaches a provider, so a hard limit costs nothing to enforce."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      {anomaly && (
        <Card
          variant={anomaly.is_anomalous ? "warm" : "subtle"}
          className="mb-6 p-5"
        >
          <div className="flex flex-wrap items-start gap-3">
            <Badge tone={anomaly.is_anomalous ? "warn" : "neutral"}>
              {anomaly.is_anomalous ? "Unusual spend" : "Spend normal"}
            </Badge>
            <div className="min-w-0 flex-1">
              <p className="text-xs font-medium leading-relaxed text-[var(--color-muted)]">
                {anomaly.explanation}
              </p>
              <dl className="mt-3 flex flex-wrap gap-x-8 gap-y-2">
                <div>
                  <dt className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-muted-light)]">
                    Today
                  </dt>
                  <dd className="font-mono text-sm font-bold text-[var(--color-ink)]">
                    {formatUsd(anomaly.observed_mc)}
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-muted-light)]">
                    Usual daily
                  </dt>
                  <dd className="font-mono text-sm font-bold text-[var(--color-muted)]">
                    {formatUsd(anomaly.baseline_mean_mc)}
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-muted-light)]">
                    Deviation
                  </dt>
                  <dd className="font-mono text-sm font-bold text-[var(--color-muted)]">
                    {anomaly.z_score.toFixed(1)}&sigma;
                  </dd>
                </div>
              </dl>
              <div className="mt-4 max-w-xs">
                <DisclosureMeter
                  label="Today vs. usual daily spend"
                  tone={anomaly.is_anomalous ? "pending" : "verdict"}
                  committedPct={
                    anomaly.observed_mc > 0
                      ? Math.min(100, (Math.min(anomaly.observed_mc, anomaly.baseline_mean_mc) / anomaly.observed_mc) * 100)
                      : 100
                  }
                  reservedPct={
                    anomaly.observed_mc > anomaly.baseline_mean_mc
                      ? ((anomaly.observed_mc - anomaly.baseline_mean_mc) / anomaly.observed_mc) * 100
                      : 0
                  }
                />
                <p className="mt-1 text-[10px] text-[var(--color-muted-light)]">
                  Solid = normal range. Hatched = spend above the usual baseline.
                </p>
              </div>
            </div>
          </div>
        </Card>
      )}

      <Card className="mb-8 p-5">
        <form onSubmit={handleCreate} className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-3">
            <div>
              <label
                htmlFor="scope"
                className="block text-sm font-medium text-[var(--color-muted)]"
              >
                Applies to
              </label>
              <select
                id="scope"
                value={scope}
                onChange={(event) => setScope(event.target.value)}
                className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
              >
                <option value="org">Whole organisation</option>
                {teams.map((team) => (
                  <option key={team.id} value={team.id}>
                    {team.name}
                  </option>
                ))}
              </select>
            </div>

            <div>
              <label
                htmlFor="period"
                className="block text-sm font-medium text-[var(--color-muted)]"
              >
                Period
              </label>
              <select
                id="period"
                value={period}
                onChange={(event) => setPeriod(event.target.value)}
                className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm capitalize text-[var(--color-ink)]"
              >
                {PERIODS.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
            </div>

            <div>
              <label
                htmlFor="limit"
                className="block text-sm font-medium text-[var(--color-muted)]"
              >
                Limit (USD)
              </label>
              <input
                id="limit"
                type="number"
                min="1"
                step="1"
                value={limitUsd}
                onChange={(event) => setLimitUsd(event.target.value)}
                className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 font-mono text-sm text-[var(--color-ink)]"
              />
            </div>
          </div>

          <label className="flex items-start gap-2.5">
            <input
              type="checkbox"
              checked={hardLimit}
              onChange={(event) => setHardLimit(event.target.checked)}
              className="mt-0.5 h-3.5 w-3.5 accent-[var(--color-accent)]"
            />
            <span className="text-xs font-medium leading-relaxed text-[var(--color-muted)]">
              <span className="font-bold text-[var(--color-ink)]">Hard limit.</span> Reject requests
              with HTTP 402 once the limit is reached. Leave this off to keep serving and
              only record the breach — safer for production traffic, more expensive when
              something goes wrong.
            </span>
          </label>

          <div className="flex justify-end">
            <Button type="submit" disabled={saving}>
              {saving ? "Creating…" : "Create budget"}
            </Button>
          </div>
        </form>
      </Card>

      {loading ? (
        <Card className="p-10 text-center text-xs font-bold text-[var(--color-muted-light)]">
          Loading budgets…
        </Card>
      ) : budgets.length === 0 ? (
        <EmptyState
          title="No budgets set"
          description="Spend is currently uncapped. A monthly organisation budget is the single cheapest piece of protection you can add — it costs nothing to enforce and turns an unbounded bill into a bounded one."
        />
      ) : (
        <TableShell>
          <thead>
            <tr>
              <Th>Applies to</Th>
              <Th>Period</Th>
              <Th align="right">Limit</Th>
              <Th>Enforcement</Th>
              <Th align="right">
                  <span className="sr-only">Actions</span>
                </Th>
            </tr>
          </thead>
          <tbody>
            {budgets.map((budget) => (
              <tr key={budget.id}>
                <Td>{scopeLabel(budget)}</Td>
                <Td muted>
                  <span className="capitalize">{budget.period}</span>
                </Td>
                <Td align="right" mono>
                  {formatUsd(budget.limit_mc)}
                </Td>
                <Td>
                  <Badge tone={budget.hard_limit ? "danger" : "neutral"} size="sm">
                    {budget.hard_limit ? "hard — rejects" : "soft — records"}
                  </Badge>
                </Td>
                <Td align="right">
                  <Button variant="danger" onClick={() => handleDelete(budget)}>
                    Remove
                  </Button>
                </Td>
              </tr>
            ))}
          </tbody>
        </TableShell>
      )}
    </>
  );
}
