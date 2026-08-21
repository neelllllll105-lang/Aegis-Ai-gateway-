"use client";

import { useMemo, useState } from "react";
import { formatUsd, savingsSharePercent } from "@/lib/format";

/**
 * The pricing calculator.
 *
 * # Why the assumptions are stated on the page
 *
 * A calculator that produces a large number without saying how is marketing, and a
 * developer evaluating an infrastructure product will discount it accordingly. Every
 * input to the estimate is shown and adjustable, and the conservative end of the range is
 * the one displayed by default.
 *
 * The savings rate is capped well below the 90% headline for the same reason: 90% is
 * achievable on a workload that is overwhelmingly simple repeated queries, and presenting
 * it as typical would set an expectation the product then fails to meet.
 */

/** Plans with a subscription and a savings share. */
const PLANS = [
  { id: "pro", label: "Pro", monthlyUsd: 29, seats: "1 seat" },
  { id: "team", label: "Team", monthlyUsd: 299, seats: "up to 10 seats" },
  { id: "enterprise", label: "Enterprise", monthlyUsd: 2000, seats: "unlimited" },
] as const;

type PlanId = (typeof PLANS)[number]["id"];

/**
 * How much of a bill Aegis can realistically remove, by workload shape.
 *
 * These are the honest middle of what the routing and caching mechanisms deliver, not the
 * best case. A workload of long, novel, reasoning-heavy requests genuinely cannot be
 * optimised much — saying so here is more useful than a number the product cannot hit.
 */
const WORKLOADS = [
  {
    id: "mixed",
    label: "Mixed application traffic",
    description: "A typical product: some lookups, some generation, some analysis.",
    savingsRate: 0.55,
  },
  {
    id: "simple",
    label: "Mostly short, repetitive requests",
    description: "Classification, extraction, lookups. Caches and routes well.",
    savingsRate: 0.78,
  },
  {
    id: "complex",
    label: "Mostly long-context reasoning",
    description: "Agents, code analysis, deep research. Little room to optimise.",
    savingsRate: 0.22,
  },
] as const;

type WorkloadId = (typeof WORKLOADS)[number]["id"];

export function SavingsCalculator() {
  const [monthlySpend, setMonthlySpend] = useState(2_000);
  const [plan, setPlan] = useState<PlanId>("pro");
  const [workload, setWorkload] = useState<WorkloadId>("mixed");

  const result = useMemo(() => {
    const shape = WORKLOADS.find((w) => w.id === workload) ?? WORKLOADS[0];
    const planDetails = PLANS.find((p) => p.id === plan) ?? PLANS[0];

    // Everything in micro-cents, mirroring the gateway, so the arithmetic shown here is
    // the same arithmetic that produces an invoice.
    const spendMc = Math.round(monthlySpend * 1_000_000);
    const grossSavingsMc = Math.round(spendMc * shape.savingsRate);
    const shareBp = savingsSharePercent(plan) * 100;
    const feeMc = Math.round((grossSavingsMc * shareBp) / 10_000);
    const subscriptionMc = planDetails.monthlyUsd * 1_000_000;
    const totalCostMc = subscriptionMc + feeMc;
    const netSavingMc = grossSavingsMc - totalCostMc;
    const newBillMc = spendMc - grossSavingsMc + totalCostMc;

    return {
      grossSavingsMc,
      feeMc,
      subscriptionMc,
      totalCostMc,
      netSavingMc,
      newBillMc,
      savingsRate: shape.savingsRate,
      worthIt: netSavingMc > 0,
    };
  }, [monthlySpend, plan, workload]);

  return (
    <div className="card p-6 sm:p-8">
      <div className="grid gap-8 lg:grid-cols-2">
        {/* --- Inputs --- */}
        <div className="space-y-6">
          <div>
            <label
              htmlFor="spend"
              className="block text-sm font-medium text-[var(--color-ink-muted)]"
            >
              Current monthly AI spend
            </label>
            <div className="mt-3 flex items-baseline gap-2">
              <span className="tabular text-3xl text-[var(--color-ink)]">
                ${monthlySpend.toLocaleString("en-US")}
              </span>
              <span className="text-sm text-[var(--color-ink-faint)]">/month</span>
            </div>
            <input
              id="spend"
              type="range"
              min={100}
              max={100_000}
              step={100}
              value={monthlySpend}
              onChange={(event) => setMonthlySpend(Number(event.target.value))}
              className="mt-4 w-full accent-[var(--color-accent)]"
              aria-describedby="spend-hint"
            />
            <p id="spend-hint" className="mt-1.5 text-xs text-[var(--color-ink-faint)]">
              Drag to match your current OpenAI, Anthropic, or Google bill.
            </p>
          </div>

          <fieldset>
            <legend className="text-sm font-medium text-[var(--color-ink-muted)]">
              What does your traffic look like?
            </legend>
            <div className="mt-3 space-y-2">
              {WORKLOADS.map((option) => (
                <label
                  key={option.id}
                  className={`flex cursor-pointer gap-3 rounded-[var(--radius)] border p-3 transition-colors ${
                    workload === option.id
                      ? "border-[var(--color-accent-dim)] bg-[var(--color-accent-wash)]"
                      : "border-[var(--color-line)] hover:border-[var(--color-line-strong)]"
                  }`}
                >
                  <input
                    type="radio"
                    name="workload"
                    value={option.id}
                    checked={workload === option.id}
                    onChange={() => setWorkload(option.id)}
                    className="mt-1 accent-[var(--color-accent)]"
                  />
                  <span>
                    <span className="block text-sm text-[var(--color-ink)]">
                      {option.label}
                    </span>
                    <span className="block text-xs text-[var(--color-ink-subtle)]">
                      {option.description}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>

          <fieldset>
            <legend className="text-sm font-medium text-[var(--color-ink-muted)]">
              Plan
            </legend>
            <div className="mt-3 flex flex-wrap gap-2">
              {PLANS.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => setPlan(option.id)}
                  className={`rounded-[var(--radius)] border px-3 py-1.5 text-sm transition-colors ${
                    plan === option.id
                      ? "border-[var(--color-accent-dim)] bg-[var(--color-accent-wash)] text-[var(--color-accent)]"
                      : "border-[var(--color-line)] text-[var(--color-ink-muted)] hover:border-[var(--color-line-strong)]"
                  }`}
                  aria-pressed={plan === option.id}
                >
                  {option.label}
                  <span className="ml-1.5 text-xs text-[var(--color-ink-faint)]">
                    ${option.monthlyUsd}/mo
                  </span>
                </button>
              ))}
            </div>
          </fieldset>
        </div>

        {/* --- Result --- */}
        <div className="rounded-[var(--radius-lg)] border border-[var(--color-line)] bg-[var(--color-base)] p-6">
          <div className="text-xs uppercase tracking-wide text-[var(--color-ink-subtle)]">
            Estimated new monthly bill
          </div>
          <div className="tabular mt-2 text-4xl text-[var(--color-accent)]">
            {formatUsd(result.newBillMc)}
          </div>
          <div className="mt-1 text-sm text-[var(--color-ink-subtle)]">
            down from {formatUsd(monthlySpend * 1_000_000)}
          </div>

          <dl className="mt-6 space-y-2.5 text-sm">
            <Row
              label="Provider costs avoided"
              value={`− ${formatUsd(result.grossSavingsMc)}`}
              tone="accent"
            />
            <Row
              label="Aegis subscription"
              value={`+ ${formatUsd(result.subscriptionMc)}`}
            />
            <Row
              label={`Savings share (${savingsSharePercent(plan)}%)`}
              value={`+ ${formatUsd(result.feeMc)}`}
            />
            <div className="border-t border-[var(--color-line)] pt-2.5">
              <Row
                label="You keep"
                value={formatUsd(result.netSavingMc)}
                tone={result.worthIt ? "accent" : "danger"}
                emphasis
              />
            </div>
          </dl>

          {!result.worthIt && (
            <p className="mt-4 rounded-[var(--radius)] border border-[#4a3a1c] bg-[#2a2113] p-3 text-xs text-[var(--color-warn)]">
              At this spend, the {PLANS.find((p) => p.id === plan)?.label} subscription
              costs more than it saves. The free tier or a smaller plan is the better
              choice — we would rather say so than sell you the wrong one.
            </p>
          )}

          <p className="mt-5 text-xs leading-relaxed text-[var(--color-ink-faint)]">
            Assumes {Math.round(result.savingsRate * 100)}% of provider cost is removable
            for this traffic shape, through cheaper-model routing, cache hits, and context
            compression. Your actual figure depends on your prompts — the dashboard shows
            the real number per request from day one, and the savings share is only ever
            charged on savings we actually delivered.
          </p>
        </div>
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  tone = "neutral",
  emphasis = false,
}: {
  label: string;
  value: string;
  tone?: "neutral" | "accent" | "danger";
  emphasis?: boolean;
}) {
  const colour =
    tone === "accent"
      ? "text-[var(--color-accent)]"
      : tone === "danger"
        ? "text-[var(--color-danger)]"
        : "text-[var(--color-ink-muted)]";

  return (
    <div className="flex items-baseline justify-between gap-4">
      <dt
        className={
          emphasis
            ? "text-[var(--color-ink)]"
            : "text-[var(--color-ink-subtle)]"
        }
      >
        {label}
      </dt>
      <dd className={`tabular ${colour} ${emphasis ? "text-base" : ""}`}>{value}</dd>
    </div>
  );
}
