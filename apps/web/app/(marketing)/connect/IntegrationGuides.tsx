"use client";

import { useState } from "react";
import { CodeBlock } from "@/components/ui";

/**
 * Real, provider-organized integration docs.
 *
 * Previously this section was a flat list where 4 of 5 examples were OpenAI-shaped
 * (curl to /v1/chat/completions, the OpenAI Python/Node SDKs, Cursor's "OpenAI API Key"
 * field) despite the page's own "Works with" badges above advertising Anthropic SDK
 * support — there was no Anthropic SDK guide anywhere, and no Google/Gemini guide at
 * all. Found live: a customer landing here after seeing "Anthropic SDK" in the badge
 * row had nothing to copy.
 *
 * The gateway genuinely has two request surfaces (apps/gateway/src/router.rs):
 * /v1/chat/completions (OpenAI wire shape) and /v1/messages (Anthropic wire shape,
 * accepts x-api-key or Authorization: Bearer — apps/gateway/src/routes/
 * anthropic_compat.rs). Neither restricts which *provider* actually serves the
 * request — the router dispatches on the `model` field regardless of which surface
 * received it, so any model, from any provider, is reachable from either surface. That
 * fact is the actual answer to "why does it only show OpenAI": it never had to be
 * OpenAI-only, the docs just never said so. Google/Gemini has no third native surface
 * (Google's own SDK speaks a different wire protocol neither of these emulates), so
 * Gemini models are called by naming them directly through one of the two surfaces
 * above — shown explicitly in its own tab below rather than left for someone to guess.
 */

type ProviderId = "openai" | "anthropic" | "google";

const PROVIDERS: { id: ProviderId; label: string; sub: string }[] = [
  { id: "openai", label: "OpenAI-compatible", sub: "/v1/chat/completions" },
  { id: "anthropic", label: "Anthropic", sub: "/v1/messages" },
  { id: "google", label: "Google / Gemini", sub: "via either surface" },
];

export function IntegrationGuides() {
  const [active, setActive] = useState<ProviderId>("openai");

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <div
        className="flex flex-wrap gap-1 rounded-xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-1"
        role="tablist"
        aria-label="Integration guides by provider"
      >
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            type="button"
            role="tab"
            aria-selected={active === p.id}
            onClick={() => setActive(p.id)}
            className={`flex-1 min-w-[9rem] rounded-lg px-3 py-2 text-left transition-colors ${
              active === p.id
                ? "border border-[var(--color-ink)] bg-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)]"
                : "text-[var(--color-muted)] hover:text-[var(--color-ink)]"
            }`}
          >
            <span className="block text-xs font-bold text-[var(--color-ink)]">
              {p.label}
            </span>
            <span className="block font-mono text-[10px] font-semibold text-[var(--color-muted-light)]">
              {p.sub}
            </span>
          </button>
        ))}
      </div>

      {active === "openai" && <OpenAiPanel />}
      {active === "anthropic" && <AnthropicPanel />}
      {active === "google" && <GooglePanel />}
    </div>
  );
}

function PanelIntro({ children }: { children: React.ReactNode }) {
  return (
    <p className="rounded-xl border border-[var(--color-ink)] bg-[var(--color-surface2)] px-4 py-3 text-xs font-medium leading-relaxed text-[var(--color-muted)]">
      {children}
    </p>
  );
}

function OpenAiPanel() {
  return (
    <div className="space-y-3">
      <PanelIntro>
        Any model Aegis routes to is reachable here — OpenAI, Anthropic, or Google — the
        &quot;OpenAI-compatible&quot; name describes the request/response shape, not which
        provider ends up serving it. Change only the <code>model</code> field to switch
        providers.
      </PanelIntro>

      <GuideItem
        title="Cursor / IDE Integration"
        instruction="Settings → Models → enable OpenAI API Key, paste your aegis_sk_ key, tick Override OpenAI Base URL and set it to the URL below."
        code={`Base URL:  http://localhost:8080/v1
API Key:   aegis_sk_live_your_key_here`}
        language="text"
      />

      <GuideItem
        title="Python (openai SDK)"
        instruction="Pass base_url and your Aegis key during client initialization:"
        code={`from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="aegis_sk_live_your_key_here",
)

response = client.chat.completions.create(
    model="gpt-4o",  # or "claude-sonnet-4-5", "google/gemini-2.5-flash" — same call
    messages=[{"role": "user", "content": "Hello Aegis!"}],
)`}
        language="python"
      />

      <GuideItem
        title="TypeScript / Node.js (openai SDK)"
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
        language="typescript"
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
  );
}

function AnthropicPanel() {
  return (
    <div className="space-y-3">
      <PanelIntro>
        The real Anthropic SDK and Claude Code point here unchanged — same headers, same
        message shape, same streaming events. Just like the OpenAI-compatible surface,
        every model Aegis routes to is reachable, not only Claude.
      </PanelIntro>

      <GuideItem
        title="Claude Code CLI & Terminal"
        instruction="Export the following two environment variables before running the claude command:"
        code={`export ANTHROPIC_BASE_URL="http://localhost:8080"
export ANTHROPIC_API_KEY="aegis_sk_live_your_key_here"

claude`}
      />

      <GuideItem
        title="Python (anthropic SDK)"
        instruction="Pass base_url and your Aegis key during client initialization — no /v1 suffix, the SDK appends /v1/messages itself:"
        code={`from anthropic import Anthropic

client = Anthropic(
    base_url="http://localhost:8080",
    api_key="aegis_sk_live_your_key_here",
)

response = client.messages.create(
    model="claude-sonnet-4-5",  # or "gpt-4o", "google/gemini-2.5-flash" — same call
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello Aegis!"}],
)`}
        language="python"
      />

      <GuideItem
        title="TypeScript / Node.js (@anthropic-ai/sdk)"
        instruction="Change baseURL in your client instantiation:"
        code={`import Anthropic from "@anthropic-ai/sdk";

const client = new Anthropic({
  baseURL: "http://localhost:8080",
  apiKey: process.env.AEGIS_API_KEY,
});

const response = await client.messages.create({
  model: "claude-sonnet-4-5",
  max_tokens: 1024,
  messages: [{ role: "user", content: "Summarize this data" }],
});`}
        language="typescript"
      />

      <GuideItem
        title="Raw HTTP / cURL"
        instruction="The SDK's own auth header works directly — x-api-key or Authorization: Bearer, either is accepted:"
        code={`curl http://localhost:8080/v1/messages \\
  -H "x-api-key: aegis_sk_live_your_key_here" \\
  -H "anthropic-version: 2023-06-01" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello Aegis!"}]
  }'`}
      />
    </div>
  );
}

function GooglePanel() {
  return (
    <div className="space-y-3">
      <PanelIntro>
        There&apos;s no native Gemini SDK passthrough — Google&apos;s own SDK speaks a
        wire protocol neither surface above emulates. Call Gemini models through the
        OpenAI-compatible or Anthropic-compatible surface instead, naming the model
        directly. Same routing, same cost savings, same dashboard.
      </PanelIntro>

      <GuideItem
        title="Python (openai SDK, calling Gemini)"
        instruction="Point the OpenAI SDK at Aegis as usual, and just name a Google model:"
        code={`from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="aegis_sk_live_your_key_here",
)

response = client.chat.completions.create(
    model="google/gemini-2.5-flash",
    messages=[{"role": "user", "content": "Hello Aegis!"}],
)`}
        language="python"
      />

      <GuideItem
        title="Raw HTTP / cURL"
        instruction="Same OpenAI-shaped request, Google model id:"
        code={`curl http://localhost:8080/v1/chat/completions \\
  -H "Authorization: Bearer aegis_sk_live_your_key_here" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "google/gemini-2.5-flash",
    "messages": [{"role": "user", "content": "Hello Aegis!"}]
  }'`}
      />

      <GuideItem
        title="Not sure which Gemini model to pin?"
        instruction={`Send "auto" as the model and Aegis starts from a strong current default (currently google/gemini-2.5-flash) — normal cost-routing still applies on top of it:`}
        code={`{
  "model": "auto",
  "messages": [{"role": "user", "content": "Hello Aegis!"}]
}`}
        language="json"
      />
    </div>
  );
}

function GuideItem({
  title,
  instruction,
  code,
  language,
}: {
  title: string;
  instruction: React.ReactNode;
  code: string;
  language?: string;
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
        <CodeBlock code={code} language={language} />
      </div>
    </details>
  );
}
