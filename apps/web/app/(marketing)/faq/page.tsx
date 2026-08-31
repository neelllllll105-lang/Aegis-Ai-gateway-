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
    <div className="mx-auto max-w-5xl space-y-16 pb-16 pt-4 text-[var(--color-ink)]">
      {/* Comparison Section matching tokenator.ai/faq layout */}
      <section className="space-y-6">
        <header className="mx-auto max-w-2xl text-center">
          <div className="font-mono text-[11px] font-bold uppercase tracking-[0.2em] text-[var(--color-muted-light)]">
            How Aegis compares
          </div>
          <h1 className="mt-2 text-2xl sm:text-4xl font-bold tracking-tight text-[var(--color-ink)]">
            One control layer, not another point tool
          </h1>
          <p className="mt-3 text-xs sm:text-sm leading-relaxed text-[var(--color-muted)] font-medium">
            You don&apos;t run Aegis <i>instead</i> of your favorite tools — you run it <i>underneath</i> them. Aegis manages the whole bill — <b className="text-[var(--color-ink)]">cutting cost, protecting quality, and showing you the savings.</b>
          </p>
        </header>

        {/* Comparison Matrix Table */}
        <div className="overflow-x-auto rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)]">
          <table className="w-full border-collapse text-left text-xs">
            <thead>
              <tr className="border-b border-[var(--color-ink)] bg-[var(--color-surface2)]">
                <th className="px-4 py-3 font-bold text-[var(--color-ink)]">Layer</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Lowers Cost</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">No Pay Twice</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Budgets</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Visibility</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Attribution</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Cuts Spend</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Shows Savings</th>
                <th className="px-3 py-3 text-center font-bold text-[var(--color-muted)]">Protects Quality</th>
              </tr>
            </thead>
            <tbody>
              {COMPARISON_DATA.map((row, idx) => (
                <tr
                  key={idx}
                  className={`border-b border-[var(--color-ink)] last:border-0 ${
                    row.isAegis ? "bg-[var(--color-surface2)] font-bold" : "hover:bg-[var(--color-surface2)]"
                  }`}
                >
                  <th className="px-4 py-3.5">
                    <span className={row.isAegis ? "text-[var(--color-ink)] font-bold text-sm" : "text-[var(--color-ink)] font-bold"}>
                      {row.layer}
                    </span>
                    <span className="block text-[10px] text-[var(--color-muted-light)] font-normal mt-0.5">
                      {row.subtext}
                    </span>
                  </th>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.lowersCost}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.noRepeatPay}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.safetyBudgets}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.visibility}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.attribution}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.cutsSpend}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.showsSavings}</td>
                  <td className="px-3 py-3.5 text-center text-sm font-bold text-[var(--color-ink)]">{row.protectsQuality}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--color-muted-light)]">
          <div className="flex items-center gap-4">
            <span><strong className="text-[var(--color-ink)]">●</strong> Core capability</span>
            <span><strong className="text-[var(--color-amber)]">◐</strong> Partial / Some</span>
            <span><strong className="text-[var(--color-line)]">○</strong> None</span>
          </div>
          <p className="italic text-[11px]">Only Aegis both actively cuts spend and provides verified savings audit receipts.</p>
        </div>
      </section>

      {/* FAQ Grid */}
      <section className="border-t border-[var(--color-ink)] pt-12">
        <header className="mx-auto max-w-2xl text-center mb-8">
          <div className="font-mono text-[11px] font-bold uppercase tracking-[0.2em] text-[var(--color-muted-light)]">
            FAQ
          </div>
          <h2 className="mt-2 text-3xl font-bold tracking-tight text-[var(--color-ink)]">
            Frequently asked questions
          </h2>
          <p className="mt-2 text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
            How intelligence, caching, and savings share billing operate.
          </p>
        </header>

        <div className="grid gap-3 sm:grid-cols-2">
          {FAQS.map((faq, idx) => (
            <details
              key={idx}
              className="group rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)] transition-all hover:border-[var(--color-accent)]"
            >
              <summary className="flex cursor-pointer list-none items-center justify-between font-bold text-[var(--color-ink)] text-sm marker:hidden">
                <span>{faq.q}</span>
                <span className="text-[var(--color-ink)] font-bold text-lg transition-transform group-open:rotate-45">
                  +
                </span>
              </summary>
              <p className="mt-3 text-xs text-[var(--color-muted)] font-medium leading-relaxed border-t border-[var(--color-ink)] pt-3">
                {faq.a}
              </p>
            </details>
          ))}
        </div>
      </section>

      {/* Help Banner */}
      <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-8 text-center shadow-[3px_3px_0_var(--shadow-color)]">
        <h3 className="text-xl font-bold text-[var(--color-ink)]">Ready to cut your inference bill?</h3>
        <p className="mt-2 text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
          Get started in under 5 minutes with zero code changes.
        </p>
        <div className="mt-5 flex justify-center gap-3">
          <Link
            href="/signup"
            className="rounded-lg bg-[var(--color-accent)] px-6 py-2.5 text-xs font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] hover:bg-[var(--color-accent-dark)] border border-[var(--color-accent-dark)]"
          >
            Request access →
          </Link>
          <Link
            href="/connect"
            className="rounded-lg border border-[var(--color-ink)] bg-[var(--color-surface)] px-5 py-2.5 text-xs font-bold text-[var(--color-ink)] shadow-[3px_3px_0_var(--shadow-color)] hover:bg-[var(--color-surface2)]"
          >
            Integration Guide
          </Link>
        </div>
      </div>
    </div>
  );
}
