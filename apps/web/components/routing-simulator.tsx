"use client";

import { useState } from "react";

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

  const complexityBarColor =
    current.complexityLabel === "Low"
      ? "bg-[#059669]"
      : current.complexityLabel === "Medium"
        ? "bg-[#D97706]"
        : "bg-[#2563EB]";

  return (
    <div className="rounded-3xl border border-[#D9CFC7] bg-white p-6 sm:p-8 shadow-xs text-black">
      {/* Scenario Selector Pills */}
      <div className="flex flex-wrap items-center gap-2 mb-6">
        <span className="text-[11px] font-black uppercase tracking-wider text-black mr-2">
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
                  ? "bg-[#22C7B2] text-[#0A1926] shadow-xs"
                  : "bg-[#EFE9E3] text-[#403B35] hover:bg-[#D9CFC7] hover:text-black"
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
          <div className="rounded-2xl border border-[#D9CFC7] bg-[#F9F8F6] p-4">
            <div className="flex items-center justify-between text-[11px] font-black uppercase tracking-wider text-black mb-1.5">
              <span>Inbound Client Prompt</span>
              <span className="font-mono text-[10px] text-[#0D9488] bg-[#22C7B2]/15 px-2 py-0.5 rounded-full border border-[#22C7B2]/40 font-bold">
                Requested: {current.requestedModel}
              </span>
            </div>
            <p className="text-xs text-[#403B35] font-mono leading-relaxed bg-white p-3 rounded-xl border border-[#D9CFC7]">
              &ldquo;{current.prompt}&rdquo;
            </p>
          </div>

          {/* Microsecond Pipeline */}
          <div className="rounded-2xl border border-[#D9CFC7] bg-[#F9F8F6] p-4 space-y-3">
            <div className="text-[11px] font-black uppercase tracking-wider text-black">
              Deterministic Microsecond Pipeline
            </div>

            <div className="grid grid-cols-3 gap-2.5">
              {/* Step 1: Complexity Scoring */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 2
                    ? "border-[#D9CFC7] bg-white shadow-xs"
                    : "border-dashed border-[#D9CFC7] opacity-40 bg-[#F9F8F6]"
                }`}
              >
                <div className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                  1. Complexity
                </div>
                <div className="mt-1 flex items-baseline justify-between">
                  <span className="text-xs font-black text-black">
                    {current.complexityLabel}
                  </span>
                  <span className="text-[10px] font-mono text-[#70685E] font-bold">
                    {(current.complexityScore * 100).toFixed(0)}%
                  </span>
                </div>
                <div className="mt-2 w-full h-1.5 bg-[#EFE9E3] rounded-full overflow-hidden">
                  <div
                    className={`h-full ${complexityBarColor} rounded-full transition-all duration-300`}
                    style={{ width: `${current.complexityScore * 100}%` }}
                  />
                </div>
              </div>

              {/* Step 2: Cache Lookup */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 3
                    ? "border-[#D9CFC7] bg-white shadow-xs"
                    : "border-dashed border-[#D9CFC7] opacity-40 bg-[#F9F8F6]"
                }`}
              >
                <div className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                  2. Semantic Cache
                </div>
                <div className="mt-1 flex items-center gap-1.5">
                  <span
                    className={`inline-block h-2.5 w-2.5 rounded-full ${
                      current.cacheStatus === "miss"
                        ? "bg-[#D97706]"
                        : "bg-[#059669]"
                    }`}
                  />
                  <span
                    className={`text-xs font-black capitalize ${
                      current.cacheStatus === "miss"
                        ? "text-[#D97706]"
                        : "text-[#059669]"
                    }`}
                  >
                    {current.cacheStatus.replace("_", " ")}
                  </span>
                </div>
                <div className="mt-1.5 text-[10px] text-[#70685E] font-medium truncate">
                  {current.cacheStatus === "miss" ? "Forward to engine" : "0.19ms instant hit"}
                </div>
              </div>

              {/* Step 3: Optimal Route */}
              <div
                className={`rounded-xl border p-3.5 transition-all ${
                  activeStep >= 4
                    ? "border-[#22C7B2]/50 bg-[#22C7B2]/10 shadow-xs"
                    : "border-dashed border-[#D9CFC7] opacity-40 bg-[#F9F8F6]"
                }`}
              >
                <div className="text-[10px] font-black uppercase tracking-wider text-[#0D9488]">
                  3. Optimal Model
                </div>
                <div className="mt-1 text-xs font-black text-black truncate">
                  {current.routedModel.split("/")[1] || current.routedModel}
                </div>
                <div className="mt-1.5 text-[10px] font-bold text-[#0D9488]">
                  {current.qualityPreserved}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Right column: Auditable Live Receipt */}
        <div className="lg:col-span-5 flex flex-col justify-between rounded-2xl border border-[#38332D] bg-[#141210] text-[#F9F8F6] p-6 shadow-sm">
          <div>
            {/* Window control header */}
            <div className="flex items-center justify-between border-b border-[#2E2A25] pb-3 mb-4">
              <div className="flex items-center gap-2">
                <div className="flex items-center gap-1.5">
                  <span className="w-2.5 h-2.5 rounded-full bg-[#FF5F56] border border-[#E0443E]" />
                  <span className="w-2.5 h-2.5 rounded-full bg-[#FFBD2E] border border-[#DEA123]" />
                  <span className="w-2.5 h-2.5 rounded-full bg-[#27C93F] border border-[#1AAB29]" />
                </div>
                <span className="ml-1 text-[11px] font-mono uppercase tracking-wider text-[#F9F8F6] font-bold">
                  Audit Header Receipt
                </span>
              </div>
              <span className="text-[10px] font-mono bg-[#2C2823] text-[#22C7B2] px-2 py-0.5 rounded border border-[#403B33] font-bold">
                RFC-9110
              </span>
            </div>

            <div className="space-y-2 text-xs font-mono">
              <div className="flex justify-between">
                <span className="text-[#A8A29E]">X-Aegis-Requested:</span>
                <span className="text-[#F9F8F6] font-bold">{current.requestedModel}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#A8A29E]">X-Aegis-Served-By:</span>
                <span className="text-[#22C7B2] font-bold">{current.routedModel}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#A8A29E]">X-Aegis-Baseline-Cost:</span>
                <span className="text-[#F9F8F6]">{current.baselineCost}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#A8A29E]">X-Aegis-Actual-Cost:</span>
                <span className="text-[#22C7B2] font-bold">{current.actualCost}</span>
              </div>
              <div className="flex justify-between border-t border-[#2E2A25] pt-2 mt-2">
                <span className="text-[#A8A29E]">X-Aegis-Savings:</span>
                <span className="text-[#22C7B2] font-black text-sm">{current.savings} ({current.savingsPercent})</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[#A8A29E]">X-Aegis-Overhead:</span>
                <span className="text-[#F9F8F6]">{current.overhead}</span>
              </div>
            </div>
          </div>

          <div className="mt-6 pt-4 border-t border-[#2E2A25] flex items-center justify-between">
            <span className="text-[11px] text-[#A8A29E]">
              Total Latency: <strong className="text-[#F9F8F6]">{current.latency}</strong>
            </span>
            <span className="text-[10px] font-bold uppercase tracking-wider text-[#0A1926] bg-[#22C7B2] px-2.5 py-1 rounded-full shadow-2xs">
              ✓ Audited Micro-Cents
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
