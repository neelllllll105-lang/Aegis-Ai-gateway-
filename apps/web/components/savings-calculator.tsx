"use client";

import { useMemo, useState } from "react";
import { formatUsd, savingsSharePercent } from "@/lib/format";

/**
 * Enterprise AI Savings & ROI Calculator — Deskwork design system (see app/globals.css, docs/design.md).
 */

const PLANS = [
  { id: "pro", label: "Pro", monthlyUsd: 29, seats: "1 seat", share: "20%" },
  { id: "team", label: "Team", monthlyUsd: 299, seats: "Up to 10 seats", share: "15%" },
  { id: "enterprise", label: "Enterprise", monthlyUsd: 2000, seats: "Unlimited (VPC)", share: "10%" },
] as const;

type PlanId = (typeof PLANS)[number]["id"];

const WORKLOADS = [
  {
    id: "mixed",
    label: "Mixed Application Traffic",
    description: "Typical SaaS: conversational chat, search, automated actions, extraction.",
    savingsRate: 0.55,
    tag: "Standard SaaS",
  },
  {
    id: "simple",
    label: "High-Volume Repetitive / Classification",
    description: "Embeddings, categorizations, QA, lookups. Caches & routes extremely well.",
    savingsRate: 0.78,
    tag: "High Savings",
  },
  {
    id: "complex",
    label: "Deep Reasoning & Coding Agents",
    description: "Complex multi-step agents, code synthesis, long-context research.",
    savingsRate: 0.24,
    tag: "Conservative",
  },
] as const;

type WorkloadId = (typeof WORKLOADS)[number]["id"];

export function SavingsCalculator() {
  const [monthlySpend, setMonthlySpend] = useState(6_500);
  const [plan, setPlan] = useState<PlanId>("team");
  const [workload, setWorkload] = useState<WorkloadId>("mixed");

  const result = useMemo(() => {
    const shape = WORKLOADS.find((w) => w.id === workload) ?? WORKLOADS[0];
    const planDetails = PLANS.find((p) => p.id === plan) ?? PLANS[0];

    const spendMc = Math.round(monthlySpend * 1_000_000);
    const grossSavingsMc = Math.round(spendMc * shape.savingsRate);
    const shareBp = savingsSharePercent(plan) * 100;
    const feeMc = Math.round((grossSavingsMc * shareBp) / 10_000);
    const subscriptionMc = planDetails.monthlyUsd * 1_000_000;
    const totalCostMc = subscriptionMc + feeMc;
    const netSavingMc = grossSavingsMc - totalCostMc;
    const newBillMc = spendMc - grossSavingsMc + totalCostMc;

    const savingsPercentage =
      spendMc > 0 ? Math.round((netSavingMc / spendMc) * 100) : 0;
    const annualNetSavingsMc = netSavingMc * 12;

    return {
      spendMc,
      grossSavingsMc,
      feeMc,
      subscriptionMc,
      totalCostMc,
      netSavingMc,
      newBillMc,
      savingsPercentage,
      annualNetSavingsMc,
      shape,
      planDetails,
    };
  }, [monthlySpend, plan, workload]);

  return (
    <div className="rounded-3xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-6 sm:p-8 shadow-[3px_3px_0_var(--shadow-color)] text-[var(--color-ink)]">
      <div className="grid gap-8 lg:grid-cols-12">
        {/* --- Left Column: Inputs --- */}
        <div className="lg:col-span-7 space-y-6">
          {/* Monthly Spend Slider */}
          <div>
            <div className="flex items-baseline justify-between mb-2">
              <label
                htmlFor="spend-slider"
                className="text-xs font-bold uppercase tracking-wider text-[var(--color-ink)]"
              >
                Current Monthly AI Spend
              </label>
              <span className="tabular font-mono text-2xl font-bold text-[var(--color-ink)]">
                ${monthlySpend.toLocaleString()}
                <span className="text-xs font-semibold text-[var(--color-muted-light)]">/mo</span>
              </span>
            </div>

            <input
              id="spend-slider"
              type="range"
              min="500"
              max="100000"
              step="500"
              value={monthlySpend}
              onChange={(e) => setMonthlySpend(Number(e.target.value))}
              className="w-full h-2.5 bg-[var(--color-surface2)] rounded-lg appearance-none cursor-pointer accent-[var(--color-accent)]"
            />

            <div className="flex justify-between text-[10px] font-mono text-[var(--color-muted-light)] mt-1 font-bold">
              <span>$500</span>
              <span>$25k</span>
              <span>$50k</span>
              <span>$75k</span>
              <span>$100k+</span>
            </div>
          </div>

          {/* Workload Profile */}
          <div>
            <label className="block text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)] mb-2.5">
              Workload Profile
            </label>
            <div className="grid gap-2.5 sm:grid-cols-1">
              {WORKLOADS.map((option) => {
                const selected = workload === option.id;
                return (
                  <button
                    key={option.id}
                    type="button"
                    onClick={() => setWorkload(option.id)}
                    className={`flex items-start gap-3 rounded-2xl border p-3.5 text-left transition-all ${
                      selected
                        ? "border-[var(--color-accent)] bg-[var(--color-surface2)] shadow-[3px_3px_0_var(--shadow-color)]"
                        : "border-[var(--color-ink)] bg-[var(--color-surface)] hover:border-[var(--color-accent)] hover:bg-[var(--color-surface2)]"
                    }`}
                  >
                    <div
                      className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border ${
                        selected
                          ? "border-[var(--color-ink)] bg-[var(--color-ink)]"
                          : "border-[var(--color-ink)] bg-[var(--color-surface)]"
                      }`}
                    >
                      {selected && <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-surface)]" />}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-xs font-bold text-[var(--color-ink)]">
                          {option.label}
                        </span>
                        <span
                          className={`text-[10px] uppercase font-bold px-2 py-0.5 rounded-full ${
                            selected
                              ? "bg-[var(--color-accent)] text-[var(--color-surface)] border border-[var(--color-accent-dark)]"
                              : "bg-[var(--color-surface2)] text-[var(--color-muted)]"
                          }`}
                        >
                          {option.tag}
                        </span>
                      </div>
                      <p className="mt-0.5 text-xs text-[var(--color-muted)] leading-relaxed font-medium">
                        {option.description}
                      </p>
                    </div>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Plan Choice */}
          <div>
            <label className="block text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)] mb-2.5">
              Aegis Plan Tier
            </label>
            <div className="grid grid-cols-3 gap-2.5">
              {PLANS.map((option) => {
                const active = plan === option.id;
                return (
                  <button
                    key={option.id}
                    type="button"
                    onClick={() => setPlan(option.id)}
                    className={`rounded-2xl border p-3 text-center transition-all ${
                      active
                        ? "border-[var(--color-accent)] bg-[var(--color-surface2)] text-[var(--color-ink)] font-bold shadow-[3px_3px_0_var(--shadow-color)]"
                        : "border-[var(--color-ink)] bg-[var(--color-surface)] text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:bg-[var(--color-surface2)]"
                    }`}
                  >
                    <div className="text-xs font-bold text-[var(--color-ink)]">{option.label}</div>
                    <div className="mt-1 text-sm font-bold text-[var(--color-ink)]">
                      ${option.monthlyUsd}
                      <span className="text-[10px] font-normal text-[var(--color-muted-light)]">/mo</span>
                    </div>
                    <div className="mt-0.5 text-[10px] text-[var(--color-muted-light)] font-medium">
                      +{option.share} share
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        </div>

        {/* --- Right Column: Savings Report --- */}
        <div className="lg:col-span-5 flex flex-col justify-between rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-6 shadow-[3px_3px_0_var(--shadow-color)]">
          <div>
            <div className="flex items-center justify-between">
              <span className="text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)]">
                Estimated Net Savings
              </span>
              <span className="inline-flex items-center gap-1 rounded-full bg-[var(--color-positive)] px-2.5 py-0.5 text-xs font-bold text-[var(--color-surface)] border border-[var(--color-ink)] shadow-[3px_3px_0_var(--shadow-color)]">
                ~{result.savingsPercentage}% Net Reduction
              </span>
            </div>

            <div className="mt-3">
              <div className="tabular text-3xl sm:text-4xl font-bold tracking-tight text-[var(--color-ink)]">
                {formatUsd(result.netSavingMc)}
                <span className="text-xs font-bold text-[var(--color-muted)] ml-1">/ month</span>
              </div>
              <div className="mt-1.5 text-xs font-semibold text-[var(--color-muted)]">
                Equivalent to <strong className="text-[var(--color-ink)] font-bold">{formatUsd(result.annualNetSavingsMc)}</strong> in net annual recurring savings.
              </div>
            </div>

            {/* Visual Spend Comparison Bar */}
            <div className="mt-6 space-y-2">
              <div className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-ink)]">
                Monthly Bill Comparison
              </div>
              <div className="space-y-2">
                <div>
                  <div className="flex justify-between text-xs mb-1">
                    <span className="text-[var(--color-muted)] font-semibold">Direct Provider Cost</span>
                    <span className="font-bold text-[var(--color-ink)]">${monthlySpend.toLocaleString()}</span>
                  </div>
                  <div className="w-full h-3 bg-[var(--color-surface)] border border-[var(--color-ink)] rounded-full overflow-hidden">
                    <div className="h-full bg-[var(--color-accent)] rounded-full w-full" />
                  </div>
                </div>
                <div>
                  <div className="flex justify-between text-xs mb-1">
                    <span className="font-bold text-[var(--color-ink)]">With Aegis (Total Cost)</span>
                    <span className="font-bold text-[var(--color-ink)]">{formatUsd(result.newBillMc)}</span>
                  </div>
                  <div className="w-full h-3 bg-[var(--color-surface)] border border-[var(--color-ink)] rounded-full overflow-hidden">
                    <div
                      className="h-full bg-[var(--color-positive)] rounded-full transition-all duration-500"
                      style={{
                        width: `${Math.max(12, Math.min(100, (result.newBillMc / (monthlySpend * 1_000_000)) * 100))}%`,
                      }}
                    />
                  </div>
                </div>
              </div>
            </div>

            {/* Itemised Breakdown */}
            <div className="mt-6 border-t border-[var(--color-ink)] pt-4 space-y-2 text-xs">
              <div className="flex justify-between text-[var(--color-muted)]">
                <span className="font-medium">Avoided provider spend</span>
                <span className="tabular font-bold text-[var(--color-ink)]">
                  − {formatUsd(result.grossSavingsMc)}
                </span>
              </div>
              <div className="flex justify-between text-[var(--color-muted)]">
                <span className="font-medium">Aegis plan subscription</span>
                <span className="tabular font-bold text-[var(--color-ink)]">
                  + {formatUsd(result.subscriptionMc)}
                </span>
              </div>
              <div className="flex justify-between text-[var(--color-muted)]">
                <span className="font-medium">Performance share ({savingsSharePercent(plan)}%)</span>
                <span className="tabular font-bold text-[var(--color-ink)]">
                  + {formatUsd(result.feeMc)}
                </span>
              </div>
            </div>
          </div>

          <div className="mt-6 pt-4 border-t border-[var(--color-ink)] text-[11px] text-[var(--color-muted)] leading-relaxed">
            <p>
              🔒 <strong>Incentive Aligned:</strong> If routing or caching produces zero savings in a month, no performance share is billed. Every dollar is tracked in integer micro-cents.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
