"use client";

import { useState } from "react";
import { AttributionChip, Stamp, type StampTone } from "@/components/ui";

interface Scenario {
  id: string;
  name: string;
  category: string;
  prompt: string;
  requestedModel: string;
  routedModel: string;
  complexityScore: number;
  complexityLabel: "Low" | "Medium" | "High";
  cacheStatus: "exact_hit" | "semantic_hit" | "miss";
  routingReason: string;
  baselineCost: string;
  actualCost: string;
  savings: string;
  savingsPercent: string;
  latency: string;
  overhead: string;
  qualityPreserved: string;
}

const SCENARIOS: Scenario[] = [
  {
    id: "support-classification",
    name: "Customer Support Triage",
    category: "Classification",
    prompt: "Classify this inbound inquiry into Billing, Tech Support, or Feature Request: 'My card was billed twice for the annual plan.'",
    requestedModel: "openai/gpt-4o",
    routedModel: "openai/gpt-4o-mini",
    complexityScore: 0.18,
    complexityLabel: "Low",
    cacheStatus: "miss",
    routingReason: "complexity_downscale",
    baselineCost: "$0.007500",
    actualCost: "$0.000450",
    savings: "$0.007050",
    savingsPercent: "94%",
    latency: "284ms",
    overhead: "0.24ms",
    qualityPreserved: "100% Preserved",
  },
  {
    id: "semantic-cache-qa",
    name: "Enterprise Knowledge Base QA",
    category: "Semantic Cache",
    prompt: "What is our company's refund policy for subscriptions canceled within 14 days of billing?",
    requestedModel: "anthropic/claude-3-5-sonnet",
    routedModel: "aegis/cache-layer",
    complexityScore: 0.42,
    complexityLabel: "Medium",
    cacheStatus: "semantic_hit",
    routingReason: "semantic_similarity_98.4%",
    baselineCost: "$0.015000",
    actualCost: "$0.000000",
    savings: "$0.015000",
    savingsPercent: "100%",
    latency: "14ms",
    overhead: "0.19ms",
    qualityPreserved: "Exact Match",
  },
  {
    id: "code-generation",
    name: "Complex SQL Synthesis",
    category: "Deep Reasoning",
    prompt: "Write a recursive PostgreSQL CTE that calculates monthly customer cohort retention with dynamic churn windows across 3 tables.",
    requestedModel: "anthropic/claude-3-5-sonnet",
    routedModel: "anthropic/claude-3-5-sonnet",
    complexityScore: 0.91,
    complexityLabel: "High",
    cacheStatus: "miss",
    routingReason: "high_reasoning_passthrough",
    baselineCost: "$0.024000",
    actualCost: "$0.024000",
    savings: "$0.000000",
    savingsPercent: "0%",
    latency: "1,420ms",
    overhead: "0.31ms",
    qualityPreserved: "Flagship Quality",
  },
  {
    id: "data-extraction",
    name: "JSON Extraction & Schema Validation",
    category: "Extraction",
    prompt: "Extract name, invoice_id, due_date, and total_usd from this OCR invoice text into strict JSON.",
    requestedModel: "openai/gpt-4o",
    routedModel: "google/gemini-1.5-flash",
    complexityScore: 0.28,
    complexityLabel: "Low",
    cacheStatus: "miss",
    routingReason: "structured_extraction_match",
    baselineCost: "$0.010500",
    actualCost: "$0.000320",
    savings: "$0.010180",
    savingsPercent: "97%",
    latency: "210ms",
    overhead: "0.22ms",
    qualityPreserved: "Schema Validated",
  },
];

export function RoutingSimulator() {
  const [selectedId, setSelectedId] = useState<string>(SCENARIOS[0]!.id);
  const [simulating, setSimulating] = useState<boolean>(false);
  const [activeStep, setActiveStep] = useState<number>(4);

  const current = SCENARIOS.find((s) => s.id === selectedId) ?? SCENARIOS[0]!;

  function runSimulation(id: string) {
    setSelectedId(id);
    setSimulating(true);
    setActiveStep(1);

    setTimeout(() => setActiveStep(2), 150);
    setTimeout(() => setActiveStep(3), 300);
    setTimeout(() => {
      setActiveStep(4);
      setSimulating(false);
    }, 450);
  }

  const complexityTone: StampTone =
    current.complexityLabel === "Low" ? "verdict" : current.complexityLabel === "Medium" ? "pending" : "agent";
  const cacheTone: StampTone = current.cacheStatus === "miss" ? "pending" : "verdict";

  return (
    <div className="rounded-3xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] p-6 sm:p-8 shadow-[4px_4px_0_var(--shadow-color)] text-[var(--color-ink)]">
      {/* Scenario Selector Pills */}
      <div className={`flex flex-wrap items-center gap-2 mb-6 ${simulating ? "scan-bar" : ""}`}>
        <span className="text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)] mr-2">
          Select Workload:
        </span>
        {SCENARIOS.map((s) => {
          const isSelected = s.id === selectedId;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => runSimulation(s.id)}
              disabled={simulating}
              className={`rounded-lg px-3.5 py-1.5 text-xs font-bold transition-all ${
                isSelected
                  ? "bg-[var(--color-accent)] text-[var(--color-surface)] shadow-[2px_2px_0_var(--shadow-color)] border border-[var(--color-accent-dark)]"
                  : "bg-[var(--color-surface2)] text-[var(--color-muted)] hover:bg-[var(--color-line)] hover:text-[var(--color-ink)] border border-[var(--color-ink)]"
              }`}
            >
              {s.name}
            </button>
          );
        })}
      </div>

      <div className="grid gap-6 lg:grid-cols-12">
        {/* Left column: Pipeline Execution */}
        <div className="lg:col-span-7 space-y-4">
          {/* Inbound Prompt */}
          <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-4">
            <div className="flex items-center justify-between text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)] mb-1.5">
              <span>Inbound Client Prompt</span>
              <AttributionChip actor="you" label={`Requested: ${current.requestedModel}`} />
            </div>
            <p className="text-xs text-[var(--color-muted)] font-mono leading-relaxed bg-[var(--color-surface)] p-3 rounded-xl border border-[var(--color-line)]">
              &ldquo;{current.prompt}&rdquo;
            </p>
          </div>

          {/* Microsecond Pipeline */}
          <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-4 space-y-3">
            <div className="text-[11px] font-bold uppercase tracking-wider text-[var(--color-ink)]">
              Deterministic Microsecond Pipeline
            </div>

            <div className="grid grid-cols-3 gap-2.5">
              {/* Step 1: Complexity Scoring */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 2
                    ? "border-[var(--color-ink)] bg-[var(--color-surface)] shadow-[2px_2px_0_var(--shadow-color)]"
                    : "border-pending opacity-40 bg-[var(--color-surface2)]"
                }`}
              >
                <div className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-muted-light)]">
                  1. Complexity
                </div>
                <div className="mt-1.5">
                  <Stamp tone={complexityTone}>{current.complexityLabel}</Stamp>
                </div>
                <div className="mt-2 w-full h-1.5 bg-[var(--color-surface2)] rounded-full overflow-hidden border border-[var(--color-line)]">
                  <div
                    className="h-full rounded-full transition-all duration-300"
                    style={{
                      width: `${current.complexityScore * 100}%`,
                      backgroundColor:
                        complexityTone === "verdict"
                          ? "var(--color-positive)"
                          : complexityTone === "pending"
                            ? "var(--color-amber)"
                            : "var(--color-accent)",
                    }}
                  />
                </div>
              </div>

              {/* Step 2: Cache Lookup */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 3
                    ? "border-[var(--color-ink)] bg-[var(--color-surface)] shadow-[2px_2px_0_var(--shadow-color)]"
                    : "border-pending opacity-40 bg-[var(--color-surface2)]"
                }`}
              >
                <div className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-muted-light)]">
                  2. Semantic Cache
                </div>
                <div className="mt-1.5">
                  <Stamp tone={cacheTone}>{current.cacheStatus.replace("_", " ")}</Stamp>
                </div>
                <div className="mt-1.5 text-[10px] text-[var(--color-muted-light)] font-medium truncate">
                  {current.cacheStatus === "miss" ? "Forward to engine" : "0.19ms instant hit"}
                </div>
              </div>

              {/* Step 3: Optimal Route */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 4
                    ? "border-[var(--color-accent)] bg-[var(--color-accent-bg)] shadow-[2px_2px_0_var(--shadow-color)]"
                    : "border-pending opacity-40 bg-[var(--color-surface2)]"
                }`}
              >
                <div className="text-[10px] font-bold uppercase tracking-wider text-[var(--color-ink)]">
                  3. Optimal Model
                </div>
                <div className="mt-1.5">
                  <AttributionChip actor="agent" label={current.routedModel.split("/")[1] || current.routedModel} />
                </div>
                <div className="mt-1.5 text-[10px] font-bold text-[var(--color-ink)]">
                  {current.qualityPreserved}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Right column: Auditable Live Receipt */}
        <div className="lg:col-span-5 flex flex-col justify-between rounded-2xl border-[1.5px] border-[#3C3324] bg-[#201B14] text-[#F7F1E4] p-6 shadow-[4px_4px_0_var(--shadow-color)]">
          <div>
            {/* Window control header */}
            <div className="flex items-center justify-between border-b border-[#3C3324] pb-3 mb-4">
              <div className="flex items-center gap-2">
                <div className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-[#A8341E] border border-[#822712]" />
                  <span className="w-2.5 h-2.5 rounded-full bg-[#B8912E] border border-[#93630F]" />
                  <span className="w-2.5 h-2.5 rounded-full bg-[#5B8A4E] border border-[#3F6B34]" />
                </div>
                <span className="ml-1 text-[11px] font-mono uppercase tracking-wider text-[#F7F1E4] font-bold">
                  Audit Header Receipt
                </span>
              </div>
              <span className="text-[10px] font-mono bg-[#3C3324] text-[#D9855E] px-2 py-0.5 rounded border border-[#4A3F2C] font-bold">
                RFC-9110
              </span>
            </div>

            <div className="space-y-2 text-xs font-mono">
              <div className="flex justify-between">
                <span className="text-[#B8AC8E]">X-Aegis-Requested:</span>
                <span className="text-[#F7F1E4] font-bold">{current.requestedModel}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#B8AC8E]">X-Aegis-Served-By:</span>
                <span className="text-[#D9855E] font-bold">{current.routedModel}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#B8AC8E]">X-Aegis-Baseline-Cost:</span>
                <span className="text-[#F7F1E4]">{current.baselineCost}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#B8AC8E]">X-Aegis-Actual-Cost:</span>
                <span className="text-[#D9855E] font-bold">{current.actualCost}</span>
              </div>
              <div className="flex justify-between border-t border-[#3C3324] pt-2 mt-2">
                <span className="text-[#B8AC8E]">X-Aegis-Savings:</span>
                <span className="text-[#D9855E] font-bold text-sm">{current.savings} ({current.savingsPercent})</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#B8AC8E]">X-Aegis-Overhead:</span>
                <span className="text-[#F7F1E4]">{current.overhead}</span>
              </div>
            </div>
          </div>

          <div className="mt-6 pt-4 border-t border-[#3C3324] flex items-center justify-between">
            <span className="text-[11px] text-[#B8AC8E]">
              Total Latency: <strong className="text-[#F7F1E4]">{current.latency}</strong>
            </span>
            <span className="text-[10px] font-bold uppercase tracking-wider text-[#F7F1E4] bg-[#A8341E] px-2.5 py-1 rounded-full">
              ✓ Audited Micro-Cents
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
