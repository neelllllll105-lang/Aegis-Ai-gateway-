"use client";

import { useMemo, useState } from "react";
import { formatUsd, savingsSharePercent } from "@/lib/format";

/**
 * Enterprise AI Savings & ROI Calculator — 4-Tier Neutral Palette (#F9F8F6, #EFE9E3, #D9CFC7, #C9B59C) + Teal.
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
    <div className="rounded-3xl border border-[#D9CFC7] bg-white p-6 sm:p-8 shadow-xs text-black">
      <div className="grid gap-8 lg:grid-cols-12">
        {/* --- Left Column: Inputs --- */}
        <div className="lg:col-span-7 space-y-6">
          {/* Monthly Spend Slider */}
          <div>
            <div className="flex items-baseline justify-between mb-2">
              <label
                htmlFor="spend-slider"
                className="text-xs font-black uppercase tracking-wider text-black"
              >
                Current Monthly AI Spend
              </label>
              <span className="tabular font-mono text-2xl font-black text-black">
                ${monthlySpend.toLocaleString()}
                <span className="text-xs font-semibold text-[#70685E]">/mo</span>
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
              className="w-full h-2.5 bg-[#EFE9E3] rounded-lg appearance-none cursor-pointer accent-[#22C7B2]"
            />

            <div className="flex justify-between text-[10px] font-mono text-[#70685E] mt-1 font-bold">
              <span>$500</span>
              <span>$25k</span>
              <span>$50k</span>
              <span>$75k</span>
              <span>$100k+</span>
            </div>
          </div>

          {/* Workload Profile */}
          <div>
            <label className="block text-[11px] font-black uppercase tracking-wider text-black mb-2.5">
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
                        ? "border-[#22C7B2] bg-[#22C7B2]/10 shadow-xs"
                        : "border-[#D9CFC7] bg-white hover:border-[#C9B59C] hover:bg-[#F9F8F6]"
                    }`}
                  >
                    <div
                      className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border ${
                        selected
                          ? "border-[#22C7B2] bg-[#22C7B2]"
                          : "border-[#D9CFC7] bg-white"
                      }`}
                    >
                      {selected && <span className="h-1.5 w-1.5 rounded-full bg-[#0A1926]" />}
                    </div>
                    <div className="flex-1">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-xs font-black text-black">
                          {option.label}
                        </span>
                        <span
                          className={`text-[10px] uppercase font-black px-2 py-0.5 rounded-full ${
                            selected
                              ? "bg-[#22C7B2] text-[#0A1926]"
                              : "bg-[#EFE9E3] text-[#403B35]"
                          }`}
                        >
                          {option.tag}
                        </span>
                      </div>
                      <p className="mt-0.5 text-xs text-[#403B35] leading-relaxed font-medium">
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
            <label className="block text-[11px] font-black uppercase tracking-wider text-black mb-2.5">
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
                        ? "border-[#22C7B2] bg-[#22C7B2]/10 text-[#0D9488] font-bold shadow-xs"
                        : "border-[#D9CFC7] bg-white text-[#403B35] hover:border-[#C9B59C] hover:bg-[#F9F8F6]"
                    }`}
                  >
                    <div className="text-xs font-bold text-black">{option.label}</div>
                    <div className="mt-1 text-sm font-black text-black">
                      ${option.monthlyUsd}
                      <span className="text-[10px] font-normal text-[#70685E]">/mo</span>
                    </div>
                    <div className="mt-0.5 text-[10px] text-[#70685E] font-medium">
                      +{option.share} share
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        </div>

        {/* --- Right Column: Savings Report --- */}
        <div className="lg:col-span-5 flex flex-col justify-between rounded-2xl border border-[#22C7B2]/40 bg-[#22C7B2]/10 p-6 shadow-xs">
          <div>
            <div className="flex items-center justify-between">
              <span className="text-[11px] font-black uppercase tracking-wider text-[#0D9488]">
                Estimated Net Savings
              </span>
              <span className="inline-flex items-center gap-1 rounded-full bg-[#22C7B2] px-2.5 py-0.5 text-xs font-bold text-[#0A1926] shadow-xs">
                ~{result.savingsPercentage}% Net Reduction
              </span>
            </div>

            <div className="mt-3">
              <div className="tabular text-3xl sm:text-4xl font-black tracking-tight text-[#0D9488]">
                {formatUsd(result.netSavingMc)}
                <span className="text-xs font-bold text-[#403B35] ml-1">/ month</span>
              </div>
              <div className="mt-1.5 text-xs font-semibold text-[#403B35]">
                Equivalent to <strong className="text-black font-black">{formatUsd(result.annualNetSavingsMc)}</strong> in net annual recurring savings.
              </div>
            </div>

            {/* Visual Spend Comparison Bar */}
            <div className="mt-6 space-y-2">
              <div className="text-[10px] font-black uppercase tracking-wider text-black">
                Monthly Bill Comparison
              </div>
              <div className="space-y-2">
                <div>
                  <div className="flex justify-between text-xs mb-1">
                    <span className="text-[#403B35] font-semibold">Direct Provider Cost</span>
                    <span className="font-black text-black">${monthlySpend.toLocaleString()}</span>
                  </div>
                  <div className="w-full h-3 bg-[#EFE9E3] border border-[#D9CFC7] rounded-full overflow-hidden">
                    <div className="h-full bg-[#E11D48] rounded-full w-full" />
                  </div>
                </div>
                <div>
                  <div className="flex justify-between text-xs mb-1">
                    <span className="font-black text-[#0D9488]">With Aegis (Total Cost)</span>
                    <span className="font-black text-[#0D9488]">{formatUsd(result.newBillMc)}</span>
                  </div>
                  <div className="w-full h-3 bg-[#EFE9E3] border border-[#D9CFC7] rounded-full overflow-hidden">
                    <div
                      className="h-full bg-[#22C7B2] rounded-full transition-all duration-500"
                      style={{
                        width: `${Math.max(12, Math.min(100, (result.newBillMc / (monthlySpend * 1_000_000)) * 100))}%`,
                      }}
                    />
                  </div>
                </div>
              </div>
            </div>

            {/* Itemised Breakdown */}
            <div className="mt-6 border-t border-[#22C7B2]/30 pt-4 space-y-2 text-xs">
              <div className="flex justify-between text-[#403B35]">
                <span className="font-medium">Avoided provider spend</span>
                <span className="tabular font-black text-[#0D9488]">
                  − {formatUsd(result.grossSavingsMc)}
                </span>
              </div>
              <div className="flex justify-between text-[#403B35]">
                <span className="font-medium">Aegis plan subscription</span>
                <span className="tabular font-black text-black">
                  + {formatUsd(result.subscriptionMc)}
                </span>
              </div>
              <div className="flex justify-between text-[#403B35]">
                <span className="font-medium">Performance share ({savingsSharePercent(plan)}%)</span>
                <span className="tabular font-black text-black">
                  + {formatUsd(result.feeMc)}
                </span>
              </div>
            </div>
          </div>

          <div className="mt-6 pt-4 border-t border-[#22C7B2]/30 text-[11px] text-[#403B35] leading-relaxed">
            <p>
              🔒 <strong>Incentive Aligned:</strong> If routing or caching produces zero savings in a month, no performance share is billed. Every dollar is tracked in integer micro-cents.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
