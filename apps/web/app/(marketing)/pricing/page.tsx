import type { Metadata } from "next";
import Link from "next/link";
import { SavingsCalculator } from "@/components/savings-calculator";

export const metadata: Metadata = {
  title: "Transparent Enterprise Pricing — Aegis Gateway",
  description:
    "Predictable base subscription plus an auditable share of verified savings. If we deliver zero savings in a month, you owe zero performance fee.",
};

const TIERS = [
  {
    id: "free",
    name: "Free / Starter",
    price: "$0",
    cadence: "forever",
    share: "0% savings share",
    summary: "For side projects & initial traffic benchmarking.",
    features: [
      "10,000 requests per month",
      "Shared models (GPT-4o mini, Flash)",
      "Full savings dashboard & metrics",
      "Exact-match vector caching",
      "Community Discord support",
    ],
    cta: "Start free",
    highlighted: false,
    badge: "Free Tier",
  },
  {
    id: "pro",
    name: "Pro Developer",
    price: "$29",
    cadence: "per month",
    share: "+ 20% of verified savings",
    summary: "For production SaaS apps, indie hackers, and growing startups.",
    features: [
      "Unlimited monthly requests",
      "Bring your own provider keys",
      "Quality-aware routing classifier",
      "Multi-tier semantic caching",
      "Custom routing policies",
      "Standard email support",
    ],
    cta: "Start 14-day trial",
    highlighted: true,
    badge: "Most Popular",
  },
  {
    id: "team",
    name: "Team & Growth",
    price: "$299",
    cadence: "per month",
    share: "+ 15% of verified savings",
    summary: "For scaling engineering organizations needing governance and team budgets.",
    features: [
      "Everything in Pro",
      "Up to 10 seats & RBAC",
      "Per-team API key budget limits",
      "Slack threshold alerts & webhooks",
      "3,000 requests/minute throughput",
      "Priority engineering support",
    ],
    cta: "Start Team trial",
    highlighted: false,
    badge: "For Teams",
  },
  {
    id: "enterprise",
    name: "Enterprise & VPC",
    price: "$2,000+",
    cadence: "per month",
    share: "+ 10% of verified savings",
    summary: "For regulated healthcare, fintech, and security-first enterprises.",
    features: [
      "Everything in Team",
      "Self-hosted Docker / Kubernetes in your VPC",
      "SSO / SAML (Okta, Azure AD, Google)",
      "Zero prompt retention mode",
      "Data residency pinning (US/EU)",
      "99.99% uptime SLA & dedicated architect",
    ],
    cta: "Contact Enterprise Sales",
    highlighted: false,
    badge: "Custom VPC",
  },
] as const;

export default function PricingPage() {
  return (
    <div className="space-y-16 sm:space-y-24 pb-20 text-black">
      {/* Header */}
      <section className="mx-auto max-w-5xl px-6 pt-12 text-center">
        <div className="font-mono text-[11px] font-black uppercase tracking-[0.2em] text-[#70685E]">
          Incentive-Aligned Pricing
        </div>
        <h1 className="mt-3 text-4xl sm:text-6xl font-black tracking-tight text-black">
          You only pay us when we save you money
        </h1>
        <p className="mx-auto mt-4 max-w-2xl text-base sm:text-lg leading-relaxed text-[#403B35] font-semibold">
          A predictable base subscription for the gateway platform, plus a transparent percentage of verified savings. If a month produces zero savings, you pay zero performance fee.
        </p>
      </section>

      {/* Pricing Cards */}
      <section className="mx-auto max-w-6xl px-6">
        <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
          {TIERS.map((tier) => (
            <div
              key={tier.id}
              className={`rounded-3xl border bg-white p-6 shadow-xs flex flex-col justify-between transition-all ${
                tier.highlighted
                  ? "border-[#C9B59C] ring-2 ring-[#C9B59C] shadow-md scale-[1.02]"
                  : "border-[#D9CFC7] hover:border-black"
              }`}
            >
              <div>
                <div className="flex items-center justify-between">
                  <span
                    className={`text-[11px] font-black uppercase tracking-wider px-3 py-1 rounded-full ${
                      tier.highlighted
                        ? "bg-[#C9B59C] text-[#0A0A0A] border border-[#BFAF98]"
                        : "bg-[#EFE9E3] text-black border border-[#D9CFC7]"
                    }`}
                  >
                    {tier.badge}
                  </span>
                </div>

                {/* Card Title - Pure Jet Black */}
                <h2 className="mt-5 text-xl font-black text-black tracking-tight">
                  {tier.name}
                </h2>

                <div className="mt-3 flex items-baseline gap-1.5">
                  <span className="tabular text-4xl font-black text-black">
                    {tier.price}
                  </span>
                  <span className="text-xs text-[#70685E] font-extrabold">
                    {tier.cadence}
                  </span>
                </div>

                <div className="mt-1.5 text-xs font-black text-black">
                  {tier.share}
                </div>

                <p className="mt-4 text-xs leading-relaxed text-[#403B35] font-medium">
                  {tier.summary}
                </p>

                <div className="mt-6 border-t border-[#D9CFC7] pt-4">
                  <ul className="space-y-2.5">
                    {tier.features.map((feature) => (
                      <li
                        key={feature}
                        className="flex items-start gap-2 text-xs text-[#403B35] font-semibold"
                      >
                        <span
                          aria-hidden="true"
                          className="mt-0.5 text-black font-black shrink-0"
                        >
                          ✓
                        </span>
                        <span>{feature}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              </div>

              <div className="mt-8 pt-4 border-t border-[#D9CFC7]">
                <Link
                  href={tier.id === "enterprise" ? "/connect" : "/signup"}
                  className={`block w-full rounded-lg px-4 py-2.5 text-center text-xs font-bold transition-all ${
                    tier.highlighted
                      ? "bg-[#C9B59C] text-[#0A0A0A] shadow-xs hover:bg-[#BFAF98] border border-[#BFAF98]"
                      : "border border-black bg-white text-black hover:bg-[#EFE9E3]"
                  }`}
                >
                  {tier.cta}
                </Link>
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* Interactive Savings Calculator */}
      <section className="mx-auto max-w-5xl px-6">
        <div className="text-center max-w-2xl mx-auto mb-8">
          <div className="font-mono text-[11px] font-black uppercase tracking-[0.2em] text-[#70685E]">
            Estimate Your Invoices
          </div>
          <h2 className="mt-3 text-3xl sm:text-4xl font-black tracking-tight text-black">
            Model your net ROI and enterprise savings
          </h2>
          <p className="mt-2 text-sm text-[#403B35] font-semibold">
            Every formula is transparent, open, and auditable.
          </p>
        </div>

        <SavingsCalculator />
      </section>

      {/* The Math Behind the Savings Share */}
      <section className="mx-auto max-w-3xl px-6">
        <div className="rounded-3xl border border-[#D9CFC7] bg-white p-8 shadow-xs">
          <h2 className="text-2xl font-black tracking-tight text-black">
            How the performance savings share is audited
          </h2>

          <div className="mt-5 space-y-4 text-xs leading-relaxed text-[#403B35] font-medium">
            <p>
              For every single request routed through Aegis, we record what it would have cost on the baseline requested model vs. what it actually cost on the routed model:
            </p>
            <div className="rounded-2xl border border-[#D9CFC7] bg-[#F9F8F6] p-4 font-mono text-xs text-black font-bold leading-relaxed">
              <code>{`gross_savings = baseline_cost - actual_cost     # floored at 0
aegis_share   = gross_savings * your_plan_rate
net_savings   = gross_savings - aegis_share          # what you keep`}</code>
            </div>
            <div className="space-y-2 pt-2">
              <div className="flex items-start gap-2 font-bold text-black text-xs">
                <span className="text-black font-black">1.</span>
                <span>The performance fee can never exceed the verified savings.</span>
              </div>
              <div className="flex items-start gap-2 font-bold text-black text-xs">
                <span className="text-black font-black">2.</span>
                <span>Any month with zero savings generates exactly $0 in share fees.</span>
              </div>
              <div className="flex items-start gap-2 font-bold text-black text-xs">
                <span className="text-black font-black">3.</span>
                <span>All arithmetic is integer micro-cents, avoiding rounding drift.</span>
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
