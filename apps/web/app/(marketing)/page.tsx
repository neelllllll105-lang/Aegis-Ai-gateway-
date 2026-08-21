import Link from "next/link";
import { SavingsCalculator } from "@/components/savings-calculator";
import { CodeBlock } from "@/components/ui";

/**
 * Landing page.
 *
 * The audience is a developer who is already paying too much for AI and is suspicious of
 * anything claiming to fix it. The page is written for that reader: the integration is
 * shown in the hero rather than described, the savings claim is qualified rather than
 * shouted, and the comparison table names real competitors with their real advantages.
 *
 * A landing page that overclaims to this audience does not convert; it gets posted
 * somewhere with a sceptical comment attached.
 */

export default function LandingPage() {
  return (
    <>
      {/* --- Hero ------------------------------------------------------------ */}
      <section className="relative overflow-hidden">
        <div className="grid-bg absolute inset-0 opacity-[0.4]" aria-hidden="true" />
        <div className="relative mx-auto max-w-5xl px-6 pt-20 pb-16 sm:pt-28">
          <div className="inline-flex items-center gap-2 rounded-full border border-[var(--color-line)] bg-[var(--color-surface)] px-3 py-1 text-xs text-[var(--color-ink-subtle)]">
            <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-accent)]" />
            Sub-millisecond gateway overhead, measured and shown on every request
          </div>

          <h1 className="mt-6 max-w-3xl text-4xl leading-[1.1] tracking-tight text-[var(--color-ink)] sm:text-5xl">
            Cut your AI bill by up to 90%.
            <br />
            Keep the quality.{" "}
            <span className="text-[var(--color-accent)]">Prove it.</span>
          </h1>

          <p className="mt-5 max-w-2xl text-lg leading-relaxed text-[var(--color-ink-muted)]">
            Aegis routes every request to the cheapest model that will actually do the job,
            caches what repeats, and shows you the saving per request — with the model you
            asked for, the model we used, and what each would have cost.
          </p>

          <div className="mt-8 flex flex-wrap items-center gap-3">
            <Link
              href="/signup"
              className="rounded-[var(--radius)] bg-[var(--color-accent)] px-4 py-2 text-sm font-medium text-[#04140c] transition-colors hover:bg-[#4ee9a0]"
            >
              Start free — 10k requests/month
            </Link>
            <Link
              href="/docs"
              className="rounded-[var(--radius)] border border-[var(--color-line-strong)] px-4 py-2 text-sm text-[var(--color-ink-muted)] transition-colors hover:bg-[var(--color-raised)] hover:text-[var(--color-ink)]"
            >
              Read the docs
            </Link>
            <span className="text-sm text-[var(--color-ink-faint)]">
              No credit card. Bring your own provider keys.
            </span>
          </div>

          {/* The integration is one line. Showing it beats describing it. */}
          <div className="mt-12 max-w-2xl">
            <CodeBlock
              caption="The entire integration:"
              code={`from openai import OpenAI

client = OpenAI(
    base_url="https://api.aegis.dev/v1",   # ← the only change
    api_key="aegis_sk_...",
)

# Everything else stays exactly as it was.
client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "What is 2+2?"}],
)`}
            />
          </div>
        </div>
      </section>

      {/* --- How it works ---------------------------------------------------- */}
      <section className="border-t border-[var(--color-line)]">
        <div className="mx-auto max-w-5xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            Three mechanisms, all of them auditable
          </h2>
          <p className="mt-2 max-w-2xl text-[var(--color-ink-subtle)]">
            None of this is a black box. Every decision is recorded on the request, and you
            can turn any of it off.
          </p>

          <div className="mt-10 grid gap-6 md:grid-cols-3">
            <Mechanism
              number="01"
              title="Quality-aware routing"
              body="A classifier scores each request. Simple ones go to a cheap model; complex ones go to exactly what you asked for. We never downgrade a hard request — the savings come from the long tail of easy ones."
              detail="Conservative by default: an ambiguous request routes to the model you specified."
            />
            <Mechanism
              number="02"
              title="Exact and semantic caching"
              body="Identical requests cost nothing. Near-identical ones are matched by embedding similarity above a strict threshold, so a reworded question still hits."
              detail="Cache keys are scoped to your organisation by construction — cross-tenant hits are impossible, not merely filtered."
            />
            <Mechanism
              number="03"
              title="Context compression"
              body="Duplicate system prompts, redundant whitespace, and stale history are removed before the request leaves. Same model, shorter prompt, smaller bill."
              detail="Never touches code blocks, and never silently drops history without telling the model."
            />
          </div>
        </div>
      </section>

      {/* --- Proof ------------------------------------------------------------ */}
      <section className="border-t border-[var(--color-line)] bg-[var(--color-surface)]">
        <div className="mx-auto max-w-5xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            Every request comes with its own receipt
          </h2>
          <p className="mt-2 max-w-2xl text-[var(--color-ink-subtle)]">
            Transparency is the product. These headers are on every response, and the same
            figures appear in your dashboard and on your invoice.
          </p>

          <div className="mt-8">
            <CodeBlock
              code={`X-Aegis-Model:            openai/gpt-4o-mini
X-Aegis-Requested-Model:  openai/gpt-4o
X-Aegis-Cost:             $0.000450
X-Aegis-Baseline-Cost:    $0.007500
X-Aegis-Savings:          $0.007050
X-Aegis-Cache:            miss
X-Aegis-Routing:          complexity
X-Aegis-Latency:          842ms (overhead: 0.371ms)
X-Aegis-Request-Id:       9f1c2b3e-...`}
            />
          </div>

          <p className="mt-4 text-sm text-[var(--color-ink-faint)]">
            That overhead figure is our own added latency, measured with the provider call
            excluded. We publish it because a gateway that hides its overhead is hiding
            something.
          </p>
        </div>
      </section>

      {/* --- Comparison -------------------------------------------------------- */}
      <section className="border-t border-[var(--color-line)]">
        <div className="mx-auto max-w-5xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            How we compare
          </h2>
          <p className="mt-2 max-w-2xl text-[var(--color-ink-subtle)]">
            Including where the alternatives are genuinely better. If you need 140
            providers today, use LiteLLM.
          </p>

          <div className="mt-8 overflow-x-auto">
            <table className="w-full min-w-[720px] text-sm">
              <thead>
                <tr>
                  <th className="hairline px-3 py-2.5 text-left text-xs font-medium uppercase tracking-wide text-[var(--color-ink-subtle)]">
                    &nbsp;
                  </th>
                  {["Aegis", "Tokenator", "LiteLLM", "Portkey", "OpenRouter"].map(
                    (name) => (
                      <th
                        key={name}
                        className={`hairline px-3 py-2.5 text-left text-xs font-medium uppercase tracking-wide ${
                          name === "Aegis"
                            ? "text-[var(--color-accent)]"
                            : "text-[var(--color-ink-subtle)]"
                        }`}
                      >
                        {name}
                      </th>
                    ),
                  )}
                </tr>
              </thead>
              <tbody>
                <ComparisonRow
                  label="Gateway overhead"
                  values={["<1ms", "unpublished", "10–50ms", "unpublished", "~5ms"]}
                />
                <ComparisonRow
                  label="Quality-aware routing"
                  values={["Yes", "Yes", "No", "Partial", "No"]}
                />
                <ComparisonRow
                  label="Per-request savings audit"
                  values={["Yes", "Aggregate only", "No", "No", "No"]}
                />
                <ComparisonRow
                  label="Self-hostable"
                  values={["Yes", "No", "Yes", "No", "No"]}
                />
                <ComparisonRow
                  label="Providers"
                  values={["9, growing", "6–7", "140+", "Many", "Many"]}
                />
                <ComparisonRow
                  label="Pricing"
                  values={[
                    "$29/mo + 20% of savings",
                    "Undisclosed",
                    "Free (OSS)",
                    "$2k–10k/mo",
                    "5% markup",
                  ]}
                />
              </tbody>
            </table>
          </div>
        </div>
      </section>

      {/* --- Calculator --------------------------------------------------------- */}
      <section
        id="calculator"
        className="border-t border-[var(--color-line)] bg-[var(--color-surface)]"
      >
        <div className="mx-auto max-w-5xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            What would this actually cost you?
          </h2>
          <p className="mt-2 max-w-2xl text-[var(--color-ink-subtle)]">
            Every assumption is visible and adjustable. If the answer is that we are not
            worth it at your spend, the calculator will say so.
          </p>
          <div className="mt-8">
            <SavingsCalculator />
          </div>
        </div>
      </section>

      {/* --- FAQ ---------------------------------------------------------------- */}
      <section className="border-t border-[var(--color-line)]">
        <div className="mx-auto max-w-3xl px-6 py-16">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            Questions you should be asking
          </h2>

          <div className="mt-8 space-y-6">
            <Faq
              question="What happens if your routing picks a worse model?"
              answer="Two things protect you. First, the classifier is deliberately biased toward caution — a request it is unsure about goes to the model you asked for. Second, you can force passthrough per request with a header, or globally with a policy. That escape hatch is permanent; we will never remove it."
            />
            <Faq
              question="Do you store my prompts?"
              answer="No, not by default. We store operational metadata only: model, token counts, cost, latency, cache status. Prompt and response content is stored only if you explicitly opt in, and organisations on zero-retention mode skip caching entirely so nothing is written anywhere."
            />
            <Faq
              question="How do I know the savings numbers are real?"
              answer="Every request records the model you requested, the model we used, and what each would have cost at published provider prices. Export the itemised CSV and recompute it yourself — the arithmetic is integer micro-cents, so your total will match ours exactly."
            />
            <Faq
              question="What if you go down?"
              answer="Point your base URL back at your provider and you are running as before — there is no lock-in, no proprietary format, and your provider keys are yours. For teams that need stronger guarantees, the Enterprise plan runs Aegis inside your own VPC."
            />
            <Faq
              question="Why is there a savings share on top of a subscription?"
              answer="Because it aligns us with you. We only make money on the share when we actually reduce your bill, and the fee is never charged on a month where routing failed to save anything. If we stop delivering, that revenue stops."
            />
          </div>
        </div>
      </section>

      {/* --- CTA ----------------------------------------------------------------- */}
      <section className="border-t border-[var(--color-line)] bg-[var(--color-surface)]">
        <div className="mx-auto max-w-3xl px-6 py-16 text-center">
          <h2 className="text-2xl tracking-tight text-[var(--color-ink)]">
            One base URL. Ten minutes.
          </h2>
          <p className="mx-auto mt-2 max-w-xl text-[var(--color-ink-subtle)]">
            The free tier covers 10,000 requests a month on shared models. Enough to see
            real numbers on your own traffic before deciding anything.
          </p>
          <Link
            href="/signup"
            className="mt-7 inline-block rounded-[var(--radius)] bg-[var(--color-accent)] px-5 py-2.5 text-sm font-medium text-[#04140c] transition-colors hover:bg-[#4ee9a0]"
          >
            Create a free account
          </Link>
        </div>
      </section>
    </>
  );
}

function Mechanism({
  number,
  title,
  body,
  detail,
}: {
  number: string;
  title: string;
  body: string;
  detail: string;
}) {
  return (
    <div>
      <div className="tabular text-xs text-[var(--color-accent)]">{number}</div>
      <h3 className="mt-2 text-base font-medium text-[var(--color-ink)]">{title}</h3>
      <p className="mt-2 text-sm leading-relaxed text-[var(--color-ink-muted)]">{body}</p>
      <p className="mt-3 border-l-2 border-[var(--color-line-strong)] pl-3 text-xs leading-relaxed text-[var(--color-ink-subtle)]">
        {detail}
      </p>
    </div>
  );
}

function ComparisonRow({ label, values }: { label: string; values: string[] }) {
  return (
    <tr>
      <td className="hairline px-3 py-2.5 text-[var(--color-ink-subtle)]">{label}</td>
      {values.map((value, index) => (
        <td
          key={label + value + String(index)}
          className={`hairline px-3 py-2.5 ${
            index === 0
              ? "text-[var(--color-accent)]"
              : "text-[var(--color-ink-muted)]"
          }`}
        >
          {value}
        </td>
      ))}
    </tr>
  );
}

function Faq({ question, answer }: { question: string; answer: string }) {
  return (
    <details className="group border-b border-[var(--color-line)] pb-5">
      <summary className="cursor-pointer list-none text-base text-[var(--color-ink)] marker:hidden">
        <span className="flex items-start justify-between gap-4">
          {question}
          <span
            className="mt-1 shrink-0 text-[var(--color-ink-faint)] transition-transform group-open:rotate-45"
            aria-hidden="true"
          >
            +
          </span>
        </span>
      </summary>
      <p className="mt-3 text-sm leading-relaxed text-[var(--color-ink-muted)]">
        {answer}
      </p>
    </details>
  );
}
