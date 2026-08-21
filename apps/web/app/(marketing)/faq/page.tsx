import type { Metadata } from "next";
import Link from "next/link";

export const metadata: Metadata = {
  title: "FAQ & Comparison — Aegis Gateway",
  description:
    "How Aegis compares to gateways, routers, and observability tools. Transparent answers on pricing, zero-retention privacy, and verified savings.",
};

const COMPARISON_DATA = [
  {
    layer: "Aegis AI Gateway",
    subtext: "Intelligent control plane under your fleet",
    isAegis: true,
    lowersCost: "●",
    noRepeatPay: "●",
    safetyBudgets: "●",
    visibility: "●",
    attribution: "●",
    qualityTesting: "●",
    cutsSpend: "●",
    controlsTools: "●",
    showsSavings: "●",
    protectsQuality: "●",
  },
  {
    layer: "AI Gateways",
    subtext: "Portkey · Helicone · LiteLLM · Cloudflare",
    isAegis: false,
    lowersCost: "◐",
    noRepeatPay: "●",
    safetyBudgets: "●",
    visibility: "●",
    attribution: "◐",
    qualityTesting: "◐",
    cutsSpend: "●",
    controlsTools: "◐",
    showsSavings: "○",
    protectsQuality: "○",
  },
  {
    layer: "Intelligent Routers",
    subtext: "OpenRouter · Not Diamond",
    isAegis: false,
    lowersCost: "●",
    noRepeatPay: "◐",
    safetyBudgets: "◐",
    visibility: "◐",
    attribution: "◐",
    qualityTesting: "◐",
    cutsSpend: "○",
    controlsTools: "○",
    showsSavings: "○",
    protectsQuality: "○",
  },
  {
    layer: "Observability + Caching",
    subtext: "Langfuse · Datadog · Arize",
    isAegis: false,
    lowersCost: "○",
    noRepeatPay: "◐",
    safetyBudgets: "◐",
    visibility: "●",
    attribution: "●",
    qualityTesting: "●",
    cutsSpend: "○",
    controlsTools: "○",
    showsSavings: "○",
    protectsQuality: "○",
  },
];

const FAQS = [
  {
    q: "How is Aegis different from gateways, routers, and observability tools?",
    a: "Point tools give you one slice — a way to call many models, or a passive spend report. Aegis sits under your entire AI stack: it actively lowers the bill through microsecond deterministic downscaling and semantic caching, enforces hard budgets, protects quality, and provides RFC-9110 audited proof of savings for finance.",
  },
  {
    q: "I already use Cursor / Claude Code / OpenAI SDK — where does Aegis fit?",
    a: "You don't change your code or workflow. You simply set your tool's API base URL to Aegis (http://localhost:8080/v1 or https://api.aegis.dev/v1). Aegis handles prompt classification, semantic cache lookups, and failovers transparently behind the scenes.",
  },
  {
    q: "Can Aegis automatically use a more cost-effective model when a task is simple?",
    a: "Yes. Our sub-millisecond classifier evaluates prompt structure, ambiguity, and reasoning depth. Simple queries, categorizations, and extractions route to ultra-fast models (like GPT-4o mini or Haiku), saving up to 94% on those calls without sacrificing user experience.",
  },
  {
    q: "Will my conversation stay consistent — or could the model change mid-thread?",
    a: "You retain total control. You can lock specific models permanently, use routing hints, or let Aegis optimize only when confidence is 100% assured. High-reasoning and coding tasks are never downgraded.",
  },
  {
    q: "Do I pay twice for the same question?",
    a: "No. With our zero-latency semantic cache, duplicate or semantically identical questions return instantly from cache at $0.00 cost.",
  },
  {
    q: "Are my prompts and data private?",
    a: "Yes. Aegis supports per-tenant zero-retention: prompt and response payloads are evaluated in volatile memory and never stored to disk. We also provide VPC and on-premises deployment options.",
  },
  {
    q: "How does the savings share pricing work?",
    a: "You pay a transparent base plan fee plus a verified percentage of savings generated. For every request, we record what it would have cost on the requested model vs what it actually cost on the routed model. If we generate zero savings in a month, you pay $0 in share fees.",
  },
  {
    q: "Can I bring my own API keys (BYOK)?",
    a: "Yes. On Pro, Team, and Enterprise plans, you can bring your own Anthropic, OpenAI, or Google keys. You pay your providers directly with zero token markup.",
  },
];

export default function FaqPage() {
  return (
    <div className="mx-auto max-w-5xl space-y-16 pb-16 pt-4 text-black">
      {/* Comparison Section matching tokenator.ai/faq layout */}
      <section className="space-y-6">
        <header className="mx-auto max-w-2xl text-center">
          <div className="font-mono text-[11px] font-black uppercase tracking-[0.2em] text-[#70685E]">
            How Aegis compares
          </div>
          <h1 className="mt-2 text-2xl sm:text-4xl font-black tracking-tight text-black">
            One control layer, not another point tool
          </h1>
          <p className="mt-3 text-xs sm:text-sm leading-relaxed text-[#403B35] font-medium">
            You don&apos;t run Aegis <i>instead</i> of your favorite tools — you run it <i>underneath</i> them. Aegis manages the whole bill — <b className="text-black">cutting cost, protecting quality, and showing you the savings.</b>
          </p>
        </header>

        {/* Comparison Matrix Table */}
        <div className="overflow-x-auto rounded-2xl border border-[#D9CFC7] bg-white shadow-xs">
          <table className="w-full border-collapse text-left text-xs">
            <thead>
              <tr className="border-b border-[#D9CFC7] bg-[#EFE9E3]">
                <th className="px-4 py-3 font-black text-black">Layer</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Lowers Cost</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">No Pay Twice</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Budgets</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Visibility</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Attribution</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Cuts Spend</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Shows Savings</th>
                <th className="px-3 py-3 text-center font-bold text-[#403B35]">Protects Quality</th>
              </tr>
            </thead>
            <tbody>
              {COMPARISON_DATA.map((row, idx) => (
                <tr
                  key={idx}
                  className={`border-b border-[#D9CFC7] last:border-0 ${
                    row.isAegis ? "bg-[#EFE9E3] font-bold" : "hover:bg-[#F9F8F6]"
                  }`}
                >
                  <th className="px-4 py-3.5">
                    <span className={row.isAegis ? "text-black font-black text-sm" : "text-black font-bold"}>
                      {row.layer}
                    </span>
                    <span className="block text-[10px] text-[#70685E] font-normal mt-0.5">
                      {row.subtext}
                    </span>
                  </th>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.lowersCost}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.noRepeatPay}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.safetyBudgets}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.visibility}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.attribution}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.cutsSpend}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.showsSavings}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-black">{row.protectsQuality}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[#70685E]">
          <div className="flex items-center gap-4">
            <span><strong className="text-black">●</strong> Core capability</span>
            <span><strong className="text-[#B45309]">◐</strong> Partial / Some</span>
            <span><strong className="text-[#D9CFC7]">○</strong> None</span>
          </div>
          <p className="italic text-[11px]">Only Aegis both actively cuts spend and provides verified savings audit receipts.</p>
        </div>
      </section>

      {/* FAQ Grid */}
      <section className="border-t border-[#D9CFC7] pt-12">
        <header className="mx-auto max-w-2xl text-center mb-8">
          <div className="font-mono text-[11px] font-black uppercase tracking-[0.2em] text-[#70685E]">
            FAQ
          </div>
          <h2 className="mt-2 text-3xl font-black tracking-tight text-black">
            Frequently asked questions
          </h2>
          <p className="mt-2 text-xs sm:text-sm text-[#403B35] font-semibold">
            How intelligence, caching, and savings share billing operate.
          </p>
        </header>

        <div className="grid gap-3 sm:grid-cols-2">
          {FAQS.map((faq, idx) => (
            <details
              key={idx}
              className="group rounded-2xl border border-[#D9CFC7] bg-white p-5 shadow-xs transition-all hover:border-[#C9B59C]"
            >
              <summary className="flex cursor-pointer list-none items-center justify-between font-black text-black text-sm marker:hidden">
                <span>{faq.q}</span>
                <span className="text-black font-black text-lg transition-transform group-open:rotate-45">
                  +
                </span>
              </summary>
              <p className="mt-3 text-xs text-[#403B35] font-medium leading-relaxed border-t border-[#D9CFC7] pt-3">
                {faq.a}
              </p>
            </details>
          ))}
        </div>
      </section>

      {/* Help Banner */}
      <div className="rounded-2xl border border-[#D9CFC7] bg-[#EFE9E3] p-8 text-center shadow-xs">
        <h3 className="text-xl font-black text-black">Ready to cut your inference bill?</h3>
        <p className="mt-2 text-xs sm:text-sm text-[#403B35] font-semibold">
          Get started in under 5 minutes with zero code changes.
        </p>
        <div className="mt-5 flex justify-center gap-3">
          <Link
            href="/signup"
            className="rounded-lg bg-[#C9B59C] px-6 py-2.5 text-xs font-bold text-[#0A0A0A] shadow-xs hover:bg-[#BFAF98] border border-[#BFAF98]"
          >
            Request access →
          </Link>
          <Link
            href="/connect"
            className="rounded-lg border border-black bg-white px-5 py-2.5 text-xs font-bold text-black shadow-xs hover:bg-[#F9F8F6]"
          >
            Integration Guide
          </Link>
        </div>
      </div>
    </div>
  );
}
