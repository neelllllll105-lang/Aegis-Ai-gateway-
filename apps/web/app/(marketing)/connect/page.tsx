import type { Metadata } from "next";
import { CodeBlock } from "@/components/ui";

export const metadata: Metadata = {
  title: "Connect Anything — Aegis Gateway",
  description:
    "Drop-in replacement for OpenAI and Anthropic APIs. Point Cursor, Claude Code, Python, Node.js, or any tool at Aegis with no code changes.",
};

export default function ConnectPage() {
  return (
    <div className="space-y-16 pb-16 text-[var(--color-ink)]">
      {/* Header */}
      <section className="mx-auto max-w-2xl pt-2 text-center">
        <div className="font-mono text-[11px] font-bold uppercase tracking-[0.2em] text-[var(--color-muted-light)]">
          Connect anything
        </div>
        <h1 className="mt-2 text-balance text-3xl font-bold tracking-tight sm:text-4xl text-[var(--color-ink)]">
          Connect Anything.
        </h1>
        <p className="mt-3 text-[var(--color-muted)] text-sm sm:text-base font-semibold leading-relaxed">
          Aegis is a <b className="text-[var(--color-ink)]">drop-in replacement</b> for the OpenAI and Anthropic APIs. Point your app, IDE, or SDK at it and instantly spend less, with full governance and analytics — <b className="text-[var(--color-ink)]">with no code changes</b>.
        </p>
      </section>

      {/* Base URL & 3 Steps */}
      <section className="mx-auto max-w-3xl space-y-4">
        <div className="rounded-2xl border-2 border-[var(--color-accent)] bg-[var(--color-surface2)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
          <div className="text-[11px] font-mono font-bold uppercase tracking-wide text-[var(--color-ink)]">
            Base URL
          </div>
          <div className="mt-2 flex items-center justify-between gap-2 rounded-xl bg-[var(--color-surface)] border border-[var(--color-ink)] p-3">
            <code className="font-mono text-sm sm:text-base font-bold text-[var(--color-ink)]">
              http://localhost:8080/v1
            </code>
            <span className="rounded bg-[var(--color-surface2)] px-2.5 py-1 text-xs font-bold text-[var(--color-ink)] border border-[var(--color-ink)]">
              OpenAI &amp; Anthropic Ready
            </span>
          </div>
          <div className="mt-2 text-xs text-[var(--color-muted)] font-medium">
            Paste this one URL into any tool — OpenAI-compatible apps and Claude Code alike. That&apos;s it.
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          <StepCard
            num="1"
            title="Copy the base URL"
            desc="Set it in your tool's API base-URL field."
          />
          <StepCard
            num="2"
            title="Use your aegis key"
            desc="Generate it in your dashboard, paste as API key."
          />
          <StepCard
            num="3"
            title="Run — unchanged"
            desc="Same SDK calls, now cheaper and metered."
          />
        </div>
      </section>

      {/* Works with Badges Grid */}
      <section className="space-y-6">
        <div className="text-center">
          <div className="font-mono text-[11px] font-bold uppercase tracking-[0.18em] text-[var(--color-muted-light)]">
            Works with
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-ink)]">
            Your stack, unchanged
          </h2>
          <p className="mx-auto mt-2 max-w-xl text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
            If it can set an API base URL, it works. No SDK to adopt, no rewrite.
          </p>
        </div>

        <div className="mx-auto max-w-3xl space-y-4">
          <div>
            <div className="mb-2 text-xs font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
              IDEs &amp; editors
            </div>
            <div className="flex flex-wrap gap-2">
              {["Cursor", "Claude Code", "Continue", "Cline", "Roo", "VS Code"].map((tool) => (
                <span
                  key={tool}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--color-ink)] bg-[var(--color-surface)] px-3 py-1.5 text-xs font-bold text-[var(--color-ink)] shadow-2xs"
                >
                  <span className="text-[var(--color-ink)] font-bold">✓</span> {tool}
                </span>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-2 text-xs font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
              SDKs &amp; frameworks
            </div>
            <div className="flex flex-wrap gap-2">
              {["OpenAI SDK", "Anthropic SDK", "Python", "Node.js", "LangChain", "LlamaIndex"].map((tool) => (
                <span
                  key={tool}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--color-ink)] bg-[var(--color-surface)] px-3 py-1.5 text-xs font-bold text-[var(--color-ink)] shadow-2xs"
                >
                  <span className="text-[var(--color-ink)] font-bold">✓</span> {tool}
                </span>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-2 text-xs font-bold uppercase tracking-wide text-[var(--color-muted-light)]">
              CLI
            </div>
            <div className="flex flex-wrap gap-2">
              {["Claude CLI", "aider", "curl"].map((tool) => (
                <span
                  key={tool}
                  className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--color-ink)] bg-[var(--color-surface)] px-3 py-1.5 text-xs font-bold text-[var(--color-ink)] shadow-2xs"
                >
                  <span className="text-[var(--color-ink)] font-bold">✓</span> {tool}
                </span>
              ))}
            </div>
          </div>
        </div>
      </section>

      {/* Integration Guides (Collapsible Accordions) */}
      <section id="guides" className="scroll-mt-8 space-y-4">
        <div className="text-center">
          <div className="font-mono text-[11px] font-bold uppercase tracking-[0.18em] text-[var(--color-muted-light)]">
            Integration guides
          </div>
          <h2 className="mx-auto mt-2 max-w-2xl text-balance text-2xl font-bold tracking-tight sm:text-3xl text-[var(--color-ink)]">
            Copy-paste setup for your tool
          </h2>
        </div>

        <div className="mx-auto max-w-3xl rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-5 shadow-[3px_3px_0_var(--shadow-color)]">
          <h3 className="text-sm font-bold text-[var(--color-ink)]">
            Coding in Cursor or VS Code? Get the biggest savings.
          </h3>
          <p className="mt-1.5 text-xs sm:text-sm leading-relaxed text-[var(--color-muted)] font-medium">
            Cut your coding bill without changing how you work:
          </p>
          <ol className="mt-2 space-y-1 text-xs sm:text-sm text-[var(--color-muted)] font-semibold">
            <li>1. <b>Bring your own key</b> for Anthropic, OpenAI, or Google in Settings.</li>
            <li>2. Point your editor at <b>the model you already use</b>.</li>
            <li>3. Aegis does the rest — live dashboard itemizes your net savings.</li>
          </ol>
        </div>

        <div className="mx-auto max-w-3xl space-y-3">
          <GuideItem
            title="Cursor / IDE Integration"
            instruction="Settings → Models → enable OpenAI API Key, paste your aegis_sk_ key, tick Override OpenAI Base URL and set it to the URL below."
            code={`Base URL:  http://localhost:8080/v1
API Key:   aegis_sk_live_your_key_here`}
          />

          <GuideItem
            title="Claude Code CLI & Terminal"
            instruction="Export the following two environment variables before running the claude command:"
            code={`export ANTHROPIC_BASE_URL="http://localhost:8080"
export ANTHROPIC_API_KEY="aegis_sk_live_your_key_here"

claude`}
          />

          <GuideItem
            title="Python (OpenAI SDK)"
            instruction="Pass base_url and your Aegis key during client initialization:"
            code={`from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="aegis_sk_live_your_key_here",
)

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Hello Aegis!"}],
)`}
          />

          <GuideItem
            title="TypeScript / Node.js (OpenAI SDK)"
            instruction="Change baseURL in your client instantiation:"
            code={`import OpenAI from "openai";

const client = new OpenAI({
  baseURL: "http://localhost:8080/v1",
  apiKey: process.env.AEGIS_API_KEY,
});

const response = await client.chat.completions.create({
  model: "gpt-4o",
  messages: [{ role: "user", content: "Summarize this data" }],
});`}
          />

          <GuideItem
            title="Raw HTTP / cURL"
            instruction="Standard HTTP POST to the completions endpoint:"
            code={`curl http://localhost:8080/v1/chat/completions \\
  -H "Authorization: Bearer aegis_sk_live_your_key_here" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello Aegis!"}]
  }'`}
          />
        </div>
      </section>
    </div>
  );
}

function StepCard({ num, title, desc }: { num: string; title: string; desc: string }) {
  return (
    <div className="flex gap-3 rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] p-4 shadow-[3px_3px_0_var(--shadow-color)]">
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-[var(--color-surface2)] font-mono text-xs font-bold text-[var(--color-ink)] border border-[var(--color-ink)]">
        {num}
      </span>
      <div>
        <div className="text-sm font-bold text-[var(--color-ink)]">{title}</div>
        <div className="mt-0.5 text-xs text-[var(--color-muted)] font-medium">{desc}</div>
      </div>
    </div>
  );
}

function GuideItem({
  title,
  instruction,
  code,
}: {
  title: string;
  instruction: string;
  code: string;
}) {
  return (
    <details className="group rounded-2xl border border-[var(--color-ink)] bg-[var(--color-surface)] open:bg-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)]">
      <summary className="flex cursor-pointer list-none items-center justify-between px-5 py-4 font-bold text-[var(--color-ink)] text-sm marker:hidden">
        <span>{title}</span>
        <span className="text-[var(--color-ink)] font-bold transition-transform group-open:rotate-90">
          ▶
        </span>
      </summary>
      <div className="border-t border-[var(--color-ink)] px-5 py-4">
        <p className="mb-3 text-xs sm:text-sm text-[var(--color-muted)] font-medium leading-relaxed">
          {instruction}
        </p>
        <CodeBlock code={code} />
      </div>
    </details>
  );
}
