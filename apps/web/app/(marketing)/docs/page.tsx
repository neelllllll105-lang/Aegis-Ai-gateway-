import type { Metadata } from "next";
import { CodeBlock } from "@/components/ui";

export const metadata: Metadata = {
  title: "Documentation",
  description:
    "Point any OpenAI- or Anthropic-compatible client at Aegis by changing one base URL. Quickstarts for the OpenAI SDK, Anthropic SDK, Cursor, Claude Code, Cline, and raw HTTP.",
};

/**
 * Quickstart documentation.
 *
 * Structured around the clients people actually use rather than around our API surface.
 * Someone arriving here wants to know what to paste into their own code; a reference
 * organised by endpoint makes them do the translation themselves.
 */
export default function DocsPage() {
  return (
    <div className="mx-auto max-w-3xl px-6 py-16">
      <h1 className="text-3xl tracking-tight text-[var(--color-ink)]">Quickstart</h1>
      <p className="mt-3 text-lg leading-relaxed text-[var(--color-ink-muted)]">
        Aegis speaks the OpenAI and Anthropic APIs. Change the base URL, use an Aegis key,
        and everything else in your code stays as it is.
      </p>

      <Step number="1" title="Create a key">
        <p className="text-[var(--color-ink-muted)]">
          Sign up and create a key from the dashboard. It looks like{" "}
          <code className="tabular text-[var(--color-ink)]">aegis_sk_…</code> and is shown
          exactly once.
        </p>
      </Step>

      <Step number="2" title="Add your provider key (optional)">
        <p className="text-[var(--color-ink-muted)]">
          On the free tier your requests run on our shared models. Add your own provider
          key on the Providers page to use the full model range and settle directly with
          the provider — we never mark up their pricing.
        </p>
      </Step>

      <Step number="3" title="Point your client at Aegis">
        <div className="space-y-8">
          <ClientExample
            title="OpenAI SDK (Python)"
            code={`from openai import OpenAI

client = OpenAI(
    base_url="https://api.aegis.dev/v1",
    api_key="aegis_sk_...",
)

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "What is 2+2?"}],
)`}
          />

          <ClientExample
            title="OpenAI SDK (TypeScript)"
            code={`import OpenAI from "openai";

const client = new OpenAI({
  baseURL: "https://api.aegis.dev/v1",
  apiKey: process.env.AEGIS_API_KEY,
});

const response = await client.chat.completions.create({
  model: "gpt-4o",
  messages: [{ role: "user", content: "What is 2+2?" }],
});`}
          />

          <ClientExample
            title="Anthropic SDK (Python)"
            code={`from anthropic import Anthropic

client = Anthropic(
    base_url="https://api.aegis.dev",
    api_key="aegis_sk_...",
)

message = client.messages.create(
    model="claude-sonnet-4-5",
    max_tokens=1024,
    messages=[{"role": "user", "content": "What is 2+2?"}],
)`}
          />

          <ClientExample
            title="Claude Code"
            code={`export ANTHROPIC_BASE_URL="https://api.aegis.dev"
export ANTHROPIC_API_KEY="aegis_sk_..."

claude`}
          />

          <ClientExample
            title="Cursor"
            code={`Settings → Models → Override OpenAI Base URL

  Base URL:  https://api.aegis.dev/v1
  API Key:   aegis_sk_...`}
          />

          <ClientExample
            title="Cline / aider / any OpenAI-compatible tool"
            code={`OPENAI_API_BASE="https://api.aegis.dev/v1"
OPENAI_API_KEY="aegis_sk_..."`}
          />

          <ClientExample
            title="Raw HTTP"
            code={`curl https://api.aegis.dev/v1/chat/completions \\
  -H "Authorization: Bearer aegis_sk_..." \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "What is 2+2?"}]
  }'`}
          />
        </div>
      </Step>

      <Step number="4" title="Read the receipt">
        <p className="text-[var(--color-ink-muted)]">
          Every response carries headers describing exactly what happened.
        </p>
        <div className="mt-4">
          <CodeBlock
            code={`X-Aegis-Model:            openai/gpt-4o-mini   ← what served it
X-Aegis-Requested-Model:  openai/gpt-4o        ← what you asked for
X-Aegis-Cost:             $0.000450
X-Aegis-Baseline-Cost:    $0.007500
X-Aegis-Savings:          $0.007050
X-Aegis-Cache:            miss | exact | semantic
X-Aegis-Routing:          complexity | policy | cache | passthrough | fallback
X-Aegis-Latency:          842ms (overhead: 0.371ms)
X-Aegis-Request-Id:       9f1c2b3e-...`}
          />
        </div>
      </Step>

      <section className="mt-16 border-t border-[var(--color-line)] pt-10">
        <h2 className="text-xl tracking-tight text-[var(--color-ink)]">
          Controlling routing
        </h2>
        <p className="mt-2 text-[var(--color-ink-muted)]">
          The optimizer is conservative by default and can always be overridden.
        </p>

        <div className="mt-6 space-y-6">
          <div>
            <h3 className="text-sm font-medium text-[var(--color-ink)]">
              Force the model you asked for
            </h3>
            <p className="mt-1 text-sm text-[var(--color-ink-subtle)]">
              Per request, with a header. This escape hatch is permanent — we will never
              remove it.
            </p>
            <div className="mt-3">
              <CodeBlock code={`X-Aegis-Routing-Hint: passthrough`} />
            </div>
          </div>

          <div>
            <h3 className="text-sm font-medium text-[var(--color-ink)]">
              Always prefer the cheapest capable model
            </h3>
            <div className="mt-3">
              <CodeBlock code={`X-Aegis-Routing-Hint: cheap`} />
            </div>
          </div>

          <div>
            <h3 className="text-sm font-medium text-[var(--color-ink)]">
              Organisation-wide policies
            </h3>
            <p className="mt-1 text-sm text-[var(--color-ink-subtle)]">
              Ordered rules, first match wins. Posted to{" "}
              <code className="text-[var(--color-ink-muted)]">/api/policies</code>.
            </p>
            <div className="mt-3">
              <CodeBlock
                code={`[
  { "when": { "team": "interns" },
    "then": { "max_model_tier": "mid" } },

  { "when": { "model_requested": "claude-opus-*" },
    "then": { "passthrough": true } },

  { "when": { "complexity": "simple" },
    "then": { "model_tier": "cheap" } }
]`}
              />
            </div>
          </div>
        </div>
      </section>

      <section className="mt-14 border-t border-[var(--color-line)] pt-10">
        <h2 className="text-xl tracking-tight text-[var(--color-ink)]">Error codes</h2>
        <p className="mt-2 text-[var(--color-ink-muted)]">
          Every error carries a stable{" "}
          <code className="text-[var(--color-ink)]">type</code> field. Branch on that, not
          the message.
        </p>

        <div className="mt-6 overflow-x-auto">
          <table className="w-full min-w-[560px] text-sm">
            <thead>
              <tr>
                <th className="hairline px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-[var(--color-ink-subtle)]">
                  Status
                </th>
                <th className="hairline px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-[var(--color-ink-subtle)]">
                  Type
                </th>
                <th className="hairline px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-[var(--color-ink-subtle)]">
                  What to do
                </th>
              </tr>
            </thead>
            <tbody>
              <ErrorRow
                status="401"
                type="unauthorized"
                action="Check the key is correct and has not been revoked."
              />
              <ErrorRow
                status="402"
                type="budget_exceeded"
                action="Raise the budget or disable the hard limit in Settings."
              />
              <ErrorRow
                status="403"
                type="model_not_allowed"
                action="The model is blocked by your plan or an org policy."
              />
              <ErrorRow
                status="429"
                type="rate_limit_exceeded"
                action="Back off for the seconds given in Retry-After."
              />
              <ErrorRow
                status="502"
                type="provider_error"
                action="The upstream provider failed. The message carries their reason."
              />
              <ErrorRow
                status="503"
                type="all_providers_failed"
                action="Every candidate provider was unavailable. Retry shortly."
              />
              <ErrorRow
                status="504"
                type="provider_timeout"
                action="The provider exceeded the timeout. Retry or reduce the request size."
              />
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

function Step({
  number,
  title,
  children,
}: {
  number: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mt-12">
      <div className="flex items-baseline gap-3">
        <span className="tabular text-sm text-[var(--color-accent)]">{number}</span>
        <h2 className="text-xl tracking-tight text-[var(--color-ink)]">{title}</h2>
      </div>
      <div className="mt-4">{children}</div>
    </section>
  );
}

function ClientExample({ title, code }: { title: string; code: string }) {
  return (
    <div>
      <h3 className="mb-2 text-sm font-medium text-[var(--color-ink)]">{title}</h3>
      <CodeBlock code={code} />
    </div>
  );
}

function ErrorRow({
  status,
  type,
  action,
}: {
  status: string;
  type: string;
  action: string;
}) {
  return (
    <tr>
      <td className="tabular hairline px-3 py-2 text-[var(--color-ink-muted)]">
        {status}
      </td>
      <td className="tabular hairline px-3 py-2 text-[var(--color-accent)]">{type}</td>
      <td className="hairline px-3 py-2 text-[var(--color-ink-subtle)]">{action}</td>
    </tr>
  );
}
