import type { Metadata } from "next";
import Link from "next/link";
import { SavingsCalculator } from "@/components/savings-calculator";

export const metadata: Metadata = {
  title: "Pricing",
  description:
    "A subscription plus a share of verified savings. We only earn the share when we actually reduce your bill.",
};

const TIERS = [
  {
    id: "free",
    name: "Free",
    price: "$0",
    cadence: "forever",
    share: "No savings share",
    summary: "Enough to see real numbers on your own traffic.",
    features: [
      "10,000 requests per month",
      "Shared models (GPT-4o mini, Gemini Flash)",
      "Full savings dashboard",
      "Exact-match caching",
      "10 requests/minute",
    ],
    cta: "Start free",
    highlighted: false,
  },
  {
    id: "pro",
    name: "Pro",
    price: "$29",
    cadence: "per month",
    share: "+ 20% of verified savings",
    summary: "For an individual developer or a small production workload.",
    features: [
      "Unlimited requests",
      "Bring your own provider keys",
      "Full optimization engine",
      "Semantic caching",
      "Routing policies",
      "600 requests/minute",
    ],
    cta: "Start free, upgrade later",
    highlighted: true,
  },
  {
    id: "team",
    name: "Team",
    price: "$299",
    cadence: "per month",
    share: "+ 15% of verified savings",
    summary: "For a team that needs shared governance and per-team budgets.",
    features: [
      "Everything in Pro",
      "Up to 10 seats",
      "Roles and permissions",
      "Per-team budgets and alerts",
      "Slack notifications",
      "3,000 requests/minute",
    ],
    cta: "Start free, upgrade later",
    highlighted: false,
  },
  {
    id: "enterprise",
    name: "Enterprise",
    price: "$2,000+",
    cadence: "per month",
    share: "+ 10% of verified savings",
    summary: "For regulated environments and self-hosted deployments.",
    features: [
      "Everything in Team",
      "SSO/SAML and SCIM",
      "Self-hosted in your own VPC",
      "Data residency pinning",
      "Audit log export",
      "99.9% SLA",
    ],
    cta: "Contact us",
    highlighted: false,
  },
] as const;

export default function PricingPage() {
  return (
    <>
      <section className="mx-auto max-w-5xl px-6 pt-16 pb-10">
        <h1 className="text-3xl tracking-tight text-[var(--color-ink)]">
          You pay us a share of what we save you
        </h1>
        <p className="mt-3 max-w-2xl text-lg leading-relaxed text-[var(--color-ink-muted)]">
          A subscription for the platform, plus a percentage of savings we actually
          delivered. If a month produces no savings, there is no share to pay — the
          incentive runs the right way round.
        </p>
      </section>

      <section className="mx-auto max-w-5xl px-6 pb-16">
        <div className="grid gap-5 lg:grid-cols-4">
          {TIERS.map((tier) => (
            <div
              key={tier.id}
              className={`card flex flex-col p-6 ${
                tier.highlighted
                  ? "border-[var(--color-accent-dim)] ring-1 ring-[var(--color-accent-dim)]"
                  : ""
              }`}
            >
              {tier.highlighted && (
                <div className="mb-3 text-xs font-medium uppercase tracking-wide text-[var(--color-accent)]">
                  Most popular
                </div>
              )}

              <h2 className="text-base font-medium text-[var(--color-ink)]">
                {tier.name}
              </h2>

              <div className="mt-3 flex items-baseline gap-1.5">
                <span className="tabular text-2xl text-[var(--color-ink)]">
                  {tier.price}
                </span>
                <span className="text-xs text-[var(--color-ink-faint)]">
                  {tier.cadence}
                </span>
              </div>
              <div className="mt-1 text-xs text-[var(--color-accent)]">{tier.share}</div>

              <p className="mt-4 text-sm leading-relaxed text-[var(--color-ink-subtle)]">
                {tier.summary}
              </p>

              <ul className="mt-5 flex-1 space-y-2">
                {tier.features.map((feature) => (
                  <li
                    key={feature}
                    className="flex gap-2 text-sm text-[var(--color-ink-muted)]"
                  >
                    <span
                      aria-hidden="true"
                      className="mt-0.5 text-[var(--color-accent)]"
                    >
                      ✓
                    </span>
                    {feature}
                  </li>
                ))}
              </ul>

              <Link
                href={tier.id === "enterprise" ? "/docs" : "/signup"}
                className={`mt-6 block rounded-[var(--radius)] px-3 py-2 text-center text-sm transition-colors ${
                  tier.highlighted
                    ? "bg-[var(--color-accent)] font-medium text-[#04140c] hover:bg-[#4ee9a0]"
                    : "border border-[var(--color-line-strong)] text-[var(--color-ink-muted)] hover:bg-[var(--color-raised)] hover:text-[var(--color-ink)]"
                }`}
              >
                {tier.cta}
              </Link>
            </div>
          ))}
        </div>

        <p className="mt-6 text-sm text-[var(--color-ink-subtle)]">
          Also available: an API tier at $0.0001 per request plus provider passthrough, for
          platforms building on top of Aegis.
        </p>
      </section>

      <section className="border-t border-[var(--color-line)] bg-[var(--color-surface)]">
        <div className="mx-auto max-w-5xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            Work out your own number
          </h2>
          <p className="mt-2 max-w-2xl text-[var(--color-ink-subtle)]">
            Including whether we are worth it at your spend. The calculator will tell you
            if we are not.
          </p>
          <div className="mt-8">
            <SavingsCalculator />
          </div>
        </div>
      </section>

      <section className="border-t border-[var(--color-line)]">
        <div className="mx-auto max-w-3xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            How the savings share is calculated
          </h2>

          <div className="mt-6 space-y-4 text-[var(--color-ink-muted)]">
            <p className="leading-relaxed">
              For every request we record what it would have cost on the model you asked
              for (the <em>baseline</em>) and what it actually cost on the model we used
              (the <em>actual</em>). The difference is the gross saving.
            </p>
            <pre className="tabular overflow-x-auto rounded-[var(--radius)] border border-[var(--color-line)] bg-[var(--color-base)] p-4 text-xs">
              <code>{`gross_savings = baseline_cost − actual_cost      (floored at zero)
aegis_fee     = gross_savings × your_rate
you_keep      = gross_savings − aegis_fee`}</code>
            </pre>
            <p className="leading-relaxed">
              Three properties follow, and all three are enforced in code rather than by
              policy:
            </p>
            <ul className="space-y-2 pl-5 text-sm leading-relaxed">
              <li className="list-disc">
                The fee can never exceed the saving it is charged on.
              </li>
              <li className="list-disc">
                A month where routing cost more than the requested model would have
                produces a fee of zero. That overspend is ours, not yours.
              </li>
              <li className="list-disc">
                Every figure is integer micro-cents, so your recomputation from the
                exported CSV matches ours exactly — not approximately.
              </li>
            </ul>
          </div>
        </div>
      </section>
    </>
  );
}
