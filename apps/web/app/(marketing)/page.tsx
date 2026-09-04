import Link from "next/link";
import { SavingsCalculator } from "@/components/savings-calculator";
import { RoutingSimulator } from "@/components/routing-simulator";
import { AegisLogo } from "@/components/ui";

/**
 * Aegis Homepage — Deskwork design system: dark desk chrome, warm paper
 * content surfaces, hard offset shadows, ink structure, red as the only
 * event color. See app/globals.css and docs/design.md.
 */

export default function HomePage() {
  return (
    <div className="-mt-4 space-y-16 pb-10 text-[var(--color-paper-on-desk)]">
      {/* --- HERO SECTION --- */}
      <section className="relative overflow-hidden rounded-3xl border border-[var(--color-ink)] bg-[var(--color-surface)] px-6 py-14 text-[var(--color-ink)] sm:px-10 shadow-[3px_3px_0_var(--shadow-color)]">
        <div
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              "radial-gradient(680px 300px at 82% -10%, rgba(168,52,30,.22), transparent 60%), radial-gradient(520px 280px at -8% 120%, rgba(60,51,36,.35), transparent 60%)",
          }}
        />
        <div className="relative mx-auto max-w-3xl text-center">
          {/* Aegis Sleek Emblem with Subtle Ambient Warm Glow */}
          <div className="mx-auto mb-4 flex justify-center">
            <div
              className="relative flex h-24 w-24 sm:h-28 sm:w-28 items-center justify-center rounded-3xl bg-[var(--color-surface2)] border-2 border-[var(--color-ink)] shadow-[6px_6px_0_var(--shadow-color)] transition-transform hover:scale-105"
              style={{ filter: "drop-shadow(0 10px 25px rgba(168,52,30,.35))" }}
            >
              <AegisLogo className="w-14 h-14 text-[var(--color-ink)]" />
            </div>
          </div>

          <div className="mt-6 font-mono text-[11px] font-bold uppercase tracking-[0.2em] text-[var(--color-muted-light)]">
            The control plane for AI cost and governance
          </div>

          <h1 className="mt-3 text-balance text-3xl font-bold leading-tight tracking-tight sm:text-5xl text-[var(--color-ink)]">
            Every AI call — cheaper, governed, audited.
          </h1>

          <p className="mx-auto mt-5 max-w-xl text-[var(--color-muted)] text-base sm:text-lg font-semibold leading-relaxed">
            Cut AI spend, enforce policy, and keep an audit trail for every request — no code changes.
          </p>

          <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
            <Link
              href="/signup"
              className="rounded-lg bg-[var(--color-accent)] px-7 py-3 font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all hover:bg-[var(--color-accent-dark)] active:scale-95 border border-[var(--color-accent-dark)]"
            >
              Request access →
            </Link>
            <Link
              href="/connect"
              className="rounded-lg border border-[var(--color-ink)] bg-[var(--color-surface)] px-6 py-3 font-bold text-[var(--color-ink)] shadow-[3px_3px_0_var(--shadow-color)] transition-colors hover:border-[var(--color-accent)]"
            >
              How to use
            </Link>
          </div>

          <p className="mt-3 text-xs text-[var(--color-muted-light)] font-medium">
            Aegis is live — request access or start free on shared models. No credit card required.
          </p>

          {/* 4 Stat Metrics, paper-toned */}
          <div className="mt-10 grid grid-cols-2 gap-3 sm:grid-cols-4">
            <StatBox title="Up to 90%" label="Lower cost per request" />
            <StatBox title="0" label="Code changes to adopt" />
            <StatBox title="100%" label="Of spend governed & audited" />
            <StatBox title="~10 min" label="To your first savings estimate" />
          </div>
        </div>
      </section>

      {/* --- LIVE INTERACTIVE SIMULATOR --- */}
      <section className="space-y-4">
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            Interactive Testbed
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-2xl sm:text-3xl font-bold tracking-tight text-[var(--color-paper-on-desk)]">
            Real-time sub-millisecond routing simulator
          </h2>
          <p className="mx-auto mt-1 max-w-xl text-xs sm:text-sm text-[var(--color-muted-on-desk)] font-semibold">
            See how prompt classification, semantic cache hits, and downscaled models reduce costs down to the micro-cent.
          </p>
        </div>
        <div className="mt-4">
          <RoutingSimulator />
        </div>
      </section>

      {/* --- THE PROBLEM --- */}
      <section>
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            The problem
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-paper-on-desk)]">
            AI is powerful, expensive, and impossible to control
          </h2>
        </div>
        <div className="mt-5">
          <div className="grid gap-4 sm:grid-cols-3">
            <ProblemCard
              title="Costs run away"
              description="Every request hits a premium model at full price. Spend scales with usage and nobody can see, cap, or attribute it."
            />
            <ProblemCard
              title="No governance"
              description="One team, one runaway agent, or one bad prompt can burn the month's budget — with no hard limits and no per-client controls."
            />
            <ProblemCard
              title="Quality is a guess"
              description="“Use a cheaper model” is the obvious lever, but no one can prove it won't hurt the customer experience — so nobody pulls it."
            />
          </div>
        </div>
      </section>

      {/* --- WHAT IT IS --- */}
      <section>
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            What it is
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-paper-on-desk)]">
            One intelligent layer between your apps and any model
          </h2>
        </div>
        <div className="mt-5">
          <p className="mx-auto max-w-2xl text-center text-[var(--color-muted-on-desk)] font-semibold text-sm sm:text-base">
            Point your apps at Aegis instead of a model provider. From that moment your AI runs cheaper, stays governed, and every dollar is accounted for — across whichever models you choose to use.
          </p>

          <div className="mt-6 flex flex-col items-stretch gap-3 sm:flex-row sm:items-stretch">
            <div className="flex flex-1 flex-col items-center justify-center rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 text-center shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
                Your side
              </div>
              <div className="mt-2 font-bold text-[var(--color-ink)] text-base">Apps &amp; agents</div>
              <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">No code changes — one endpoint, one key.</p>
            </div>

            <span className="self-center text-center text-2xl text-[var(--color-paper-on-desk)] font-bold sm:px-1">→</span>

            <div className="flex flex-1 flex-col items-center justify-center rounded-2xl border-2 border-[var(--color-accent)] bg-[var(--color-surface2)] p-5 text-center shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-accent)]">
                Aegis Gateway
              </div>
              <div className="mt-2 text-base font-bold text-[var(--color-ink)]">The smart layer under your AI</div>
              <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">Lowers the bill, keeps quality, and shows you the savings.</p>
              <div className="mt-3 flex flex-wrap justify-center gap-2">
                <span className="rounded-full bg-[var(--color-accent)] px-2.5 py-1 font-mono text-[10px] font-bold text-[var(--color-surface)]">Lower cost</span>
                <span className="rounded-full bg-[var(--color-accent)] px-2.5 py-1 font-mono text-[10px] font-bold text-[var(--color-surface)]">Governance</span>
                <span className="rounded-full bg-[var(--color-accent)] px-2.5 py-1 font-mono text-[10px] font-bold text-[var(--color-surface)]">Proof</span>
              </div>
            </div>

            <span className="self-center text-center text-2xl text-[var(--color-paper-on-desk)] font-bold sm:px-1">→</span>

            <div className="flex flex-1 flex-col items-center justify-center rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 text-center shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
                Any model
              </div>
              <div className="mt-2 font-bold text-[var(--color-ink)] text-base">Your key or ours</div>
              <p className="mt-1 text-xs text-[var(--color-muted)] font-medium">Open, premium, or private — per model.</p>
            </div>
          </div>

          <p className="mx-auto mt-6 max-w-2xl text-center text-xs sm:text-sm text-[var(--color-muted-on-desk)] font-medium">
            <b className="text-[var(--color-paper-on-desk)] font-bold">Works with the tools you already use.</b> Anything with an API base-URL setting is one paste away — <b>Cursor, Continue, Cline, aider, VS Code, the OpenAI SDK</b>, and <b>Claude Code</b> (we speak the Anthropic API too). No SDK to adopt, no code to rewrite.
          </p>
        </div>
      </section>

      {/* --- HOW YOU BRING THE TOKENS --- */}
      <section>
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            How you bring the tokens
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-paper-on-desk)]">
            One engine. Two ways to pay for the models.
          </h2>
        </div>
        <div className="mt-5">
          <p className="mx-auto max-w-2xl text-center text-[var(--color-muted-on-desk)] font-semibold text-xs sm:text-sm">
            Automatic savings and governance are the same for everyone. Where the tokens come from is your choice — per workspace, and per model.
          </p>
          <div className="mt-6 grid gap-4 sm:grid-cols-2">
            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-6 shadow-[3px_3px_0_var(--shadow-color)]">
              <span className="rounded-full bg-[var(--color-surface2)] border border-[var(--color-ink)] px-3 py-1 font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-ink)]">
                Start free
              </span>
              <div className="mt-3 font-bold text-[var(--color-ink)] text-lg">Shared models, zero setup</div>
              <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
                New workspaces run on our shared models out of the box — no key, no card. Savings apply from the first call. Pick your default model any time.
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-6 shadow-[3px_3px_0_var(--shadow-color)]">
              <span className="rounded-full bg-[var(--color-accent)] px-3 py-1 font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-surface)] border border-[var(--color-accent-dark)]">
                Bring your own key
              </span>
              <div className="mt-3 font-bold text-[var(--color-ink)] text-lg">Your account, premium models — Pro</div>
              <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
                On Pro, point any model at your own Anthropic, OpenAI, or Google key and run premium models (Claude, GPT-4o, Gemini) through your account. You pay your provider directly; Aegis makes every call cheaper automatically.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* --- PRIVACY & CONTROL --- */}
      <section className="rounded-3xl border border-[var(--color-ink)] bg-[var(--color-surface)] px-6 py-12 text-[var(--color-ink)] sm:px-10 shadow-[3px_3px_0_var(--shadow-color)]">
        <div className="mx-auto max-w-3xl">
          <div className="text-center">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-light)] font-bold">
              Privacy &amp; control
            </div>
            <h2 className="mt-2 text-2xl sm:text-3xl font-bold tracking-tight text-[var(--color-ink)]">
              A walled garden — your data stays yours
            </h2>
            <p className="mx-auto mt-2 max-w-2xl text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
              Your AI traffic stays private and provably safe. For regulated and sensitive workloads, Aegis runs as a private, isolated tier — no shared pool, no third-party hop, nothing retained.
            </p>
          </div>

          <div className="mt-6 grid gap-4 sm:grid-cols-3">
            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-5">
              <span className="rounded-full px-2.5 py-0.5 font-mono text-[9px] font-bold uppercase tracking-wide bg-[var(--color-surface2)] text-[var(--color-ink)] border border-[var(--color-ink)]">
                Available today
              </span>
              <div className="mt-2.5 font-bold text-[var(--color-ink)] text-sm">Per-tenant zero-retention</div>
              <p className="mt-1.5 text-xs text-[var(--color-muted)] font-medium leading-relaxed">
                Flip a switch and Aegis stores nothing for that tenant — no saved prompts or responses. Usage metering carries no prompt or output content.
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-5">
              <span className="rounded-full px-2.5 py-0.5 font-mono text-[9px] font-bold uppercase tracking-wide bg-[var(--color-surface2)] text-[var(--color-ink)] border border-[var(--color-ink)]">
                Available today
              </span>
              <div className="mt-2.5 font-bold text-[var(--color-ink)] text-sm">Your account, not a shared pool</div>
              <p className="mt-1.5 text-xs text-[var(--color-muted)] font-medium leading-relaxed">
                Sensitive traffic runs on a direct provider or the tenant&apos;s own key — never a third-party aggregator hop. Per-tenant isolation means data never leaks.
              </p>
            </div>

            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-5">
              <span className="rounded-full px-2.5 py-0.5 font-mono text-[9px] font-bold uppercase tracking-wide bg-[var(--color-accent)] text-[var(--color-surface)] border border-[var(--color-accent-dark)]">
                Enterprise
              </span>
              <div className="mt-2.5 font-bold text-[var(--color-ink)] text-sm">DPA · data residency · self-host</div>
              <p className="mt-1.5 text-xs text-[var(--color-muted)] font-medium leading-relaxed">
                Sign a DPA, pin an EU region, or deploy Aegis inside your own cloud/VPC so nothing leaves your perimeter.
              </p>
            </div>
          </div>
        </div>
      </section>

      {/* --- THE OUTCOMES --- */}
      <section>
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            The outcomes
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-paper-on-desk)]">
            What it delivers to the business
          </h2>
        </div>
        <div className="mt-5">
          <div className="grid gap-4 sm:grid-cols-2">
            <OutcomeCard
              icon="↓"
              title="Dramatically lower AI spend"
              description="In our testing across benchmark enterprise workloads, requests cost 74–89% less than the premium baseline — with no drop in quality."
            />
            <OutcomeCard
              icon="⛨"
              title="Spend that can't run away"
              description="Hard budgets, rate limits, and per-client controls stop overspend before it happens — not after the monthly invoice lands."
            />
            <OutcomeCard
              icon="✓"
              title="Savings you can prove"
              description="A live dashboard shows exactly how much you saved versus premium-only, and confirms the cheaper path kept quality intact."
            />
            <OutcomeCard
              icon="⚿"
              title="Private, secure, compliant"
              description="Per-tenant zero-retention means prompts and outputs are never stored. Complete audit trail, DPA, and VPC options."
            />
            <OutcomeCard
              icon="⤢"
              title="Adopt in an afternoon"
              description="It speaks the standard AI-API format, so existing apps and tools work by changing one setting. No rebuild, no lock-in."
            />
            <OutcomeCard
              icon="◉"
              title="Reliable & safe by default"
              description="You keep running even through upstream provider outages, with automatic fast failover."
            />
          </div>
        </div>
      </section>

      {/* --- THE RESULT (SPEND COMPARISON) --- */}
      <section className="rounded-3xl bg-[var(--color-surface)] border border-[var(--color-ink)] px-6 py-12 text-[var(--color-ink)] sm:px-10 shadow-[3px_3px_0_var(--shadow-color)]">
        <div className="mx-auto max-w-3xl">
          <div className="text-center">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-light)] font-bold">
              The result
            </div>
            <h2 className="mt-2 text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-ink)]">
              The same workload, a fraction of the cost
            </h2>
            <p className="mx-auto mt-2 max-w-2xl text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
              Illustrative: a team spending $100k/month premium-only, after moving that traffic through Aegis.
            </p>
          </div>

          <div className="mt-8 grid gap-6 sm:grid-cols-2">
            <div>
              <div className="font-mono text-[11px] font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
                Before — premium only
              </div>
              <div className="my-1.5 text-3xl font-bold text-[var(--color-accent)]">
                $100,000 / mo
              </div>
              <div className="h-5 overflow-hidden rounded-full bg-[var(--color-surface2)] border border-[var(--color-ink)]">
                <div className="h-full rounded-full bg-[var(--color-accent)] w-full" />
              </div>
            </div>

            <div>
              <div className="font-mono text-[11px] font-bold uppercase tracking-wide text-[var(--color-ink)]">
                After — with Aegis
              </div>
              <div className="my-1.5 text-3xl font-bold text-[var(--color-positive)]">
                $18,000 / mo
              </div>
              <div className="h-5 overflow-hidden rounded-full bg-[var(--color-surface2)] border border-[var(--color-ink)]">
                <div className="h-full rounded-full bg-[var(--color-positive)] w-[18%]" />
              </div>
            </div>
          </div>

          <p className="mx-auto mt-5 max-w-2xl text-center text-xs text-[var(--color-muted-light)] font-medium">
            Every dollar of that difference is shown on an auditable savings dashboard, with quality confirmed to stay intact.
          </p>

          <div className="mt-6 rounded-xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-5 text-left">
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-ink)] font-bold">
              New · Pro
            </div>
            <h3 className="mt-1 text-lg font-bold text-[var(--color-ink)]">
              Premium quality, only-when-needed pricing
            </h3>
            <p className="mt-1.5 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
              On your own key, simple requests never pay premium prices — while the hardest work still gets your strongest model, so quality never slips. Works across Claude, OpenAI, and Gemini.
            </p>
          </div>
        </div>
      </section>

      {/* --- ROI ESTIMATOR CALCULATOR --- */}
      <section className="space-y-4">
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            Interactive Calculator
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-2xl sm:text-3xl font-bold tracking-tight text-[var(--color-paper-on-desk)]">
            Estimate your exact monthly savings
          </h2>
        </div>
        <SavingsCalculator />
      </section>

      {/* --- WHO IT'S FOR --- */}
      <section>
        <div className="text-center">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--color-muted-on-desk)] font-bold">
            Who it&apos;s for
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-paper-on-desk)]">
            Built for anyone whose costs scale with AI
          </h2>
        </div>
        <div className="mt-5">
          <div className="grid gap-4 sm:grid-cols-3">
            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-bold text-[var(--color-ink)] text-base">AI-native product teams</div>
              <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
                Ship faster and cheaper — cut inference cost without touching your product or risking quality.
              </p>
            </div>
            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-bold text-[var(--color-ink)] text-base">Enterprises</div>
              <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
                Put every team, project, and app under one governed, audited, cost-controlled AI budget.
              </p>
            </div>
            <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
              <div className="font-bold text-[var(--color-ink)] text-base">Platforms &amp; agencies</div>
              <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
                Offer AI to your customers under one governed account, with client metering and margin billing.
              </p>
            </div>
          </div>

          <div className="mt-6 rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-6">
            <h3 className="text-lg font-bold text-[var(--color-ink)]">
              Why Aegis, not a gateway or a dashboard
            </h3>
            <p className="mt-2 max-w-3xl text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
              Point tools give you one piece — a way to call many models, or a spend report. <b className="text-[var(--color-ink)]">Aegis does it all in one place: lowers the bill, keeps you in control, and proves the savings</b>.
            </p>
          </div>
        </div>
      </section>

      {/* --- FINAL CTA --- */}
      <section className="space-y-5 py-6 text-center">
        <h2 className="text-balance text-3xl font-bold tracking-tight sm:text-4xl text-[var(--color-paper-on-desk)]">
          Spend less on AI. Control all of it. Prove every dollar.
        </h2>
        <p className="mx-auto max-w-xl text-xs sm:text-sm text-[var(--color-muted-on-desk)] font-semibold">
          See it on your own traffic — request access and watch the savings and controls appear in minutes.
        </p>
        <div className="flex flex-wrap items-center justify-center gap-3">
          <Link
            href="/signup"
            className="rounded-lg bg-[var(--color-accent)] px-7 py-3 font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all hover:bg-[var(--color-accent-dark)] border border-[var(--color-accent-dark)]"
          >
            Request access →
          </Link>
        </div>
        <p className="pt-4 font-mono text-[11px] font-bold text-[var(--color-muted-on-desk)]">Aegis Gateway</p>
      </section>
    </div>
  );
}

function StatBox({ title, label }: { title: string; label: string }) {
  return (
    <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] px-4 py-4 text-left shadow-[3px_3px_0_var(--shadow-color)]">
      <div className="text-2xl sm:text-3xl font-bold tracking-tight text-[var(--color-ink)]">{title}</div>
      <div className="mt-1 font-mono text-[10px] font-bold uppercase tracking-wide text-[var(--color-muted-light)]">{label}</div>
    </div>
  );
}

function ProblemCard({ title, description }: { title: string; description: string }) {
  return (
    <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
      <div className="font-bold text-[var(--color-ink)] text-base">{title}</div>
      <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">{description}</p>
    </div>
  );
}

function OutcomeCard({ icon, title, description }: { icon: string; title: string; description: string }) {
  return (
    <div className="rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
      <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--color-surface2)] font-bold text-[var(--color-ink)] border border-[var(--color-ink)]">
        {icon}
      </div>
      <div className="mt-3 font-bold text-[var(--color-ink)] text-base">{title}</div>
      <p className="mt-1 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">{description}</p>
    </div>
  );
}
