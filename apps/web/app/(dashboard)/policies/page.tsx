"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type Policy } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
} from "@/components/ui";
import { formatRelative } from "@/lib/format";

/**
 * Routing policy management.
 *
 * A policy is the customer overriding the router. The router optimises for cost inside a
 * capability tier; a policy says "regardless of what you worked out, this class of
 * request goes here." That ordering matters and is stated on the page, because the most
 * common support question about routing is why a policy did or did not win.
 *
 * Policies are edited as presets rather than raw JSON. Raw JSON is still accepted by the
 * API for anyone scripting it, but a rules object typed by hand into a browser is a
 * reliable way to produce a policy that silently matches nothing.
 */

/** The shapes the routing engine understands, as a human would describe them. */
const PRESETS = [
  {
    id: "pin-complex",
    label: "Never downgrade complex work",
    description:
      "Requests classified complex always go to the model that was asked for. Simple and medium requests still route for cost.",
    rules: { min_complexity: "complex", action: "passthrough" },
  },
  {
    id: "force-economy",
    label: "Send simple requests to economy models",
    description:
      "Anything classified simple is served by the cheapest model in the economy tier. The largest single source of savings for most teams.",
    rules: { max_complexity: "simple", action: "route", target_tier: "economy" },
  },
  {
    id: "provider-pin",
    label: "Pin to a single provider",
    description:
      "All routing stays within one provider. Used when a data-processing agreement covers one vendor and not others.",
    rules: { action: "route", pin_provider: "openai" },
  },
  {
    id: "model-allowlist",
    label: "Restrict to an approved model list",
    description:
      "Only the named models may ever be served. Requests for anything else are rejected rather than silently substituted.",
    rules: { action: "restrict", allowed_models: ["gpt-4o-mini", "claude-haiku-4-5"] },
  },
] as const;

export default function PoliciesPage() {
  const [policies, setPolicies] = useState<Policy[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const [name, setName] = useState("");
  const [preset, setPreset] = useState<string>(PRESETS[0].id);
  const [rulesJson, setRulesJson] = useState(
    JSON.stringify(PRESETS[0].rules, null, 2),
  );
  const [jsonError, setJsonError] = useState<string | null>(null);

  async function load() {
    try {
      const response = await api.listPolicies();
      setPolicies(response.policies);
      setError(null);
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not load policies.",
      );
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  function selectPreset(id: string) {
    setPreset(id);
    const found = PRESETS.find((entry) => entry.id === id);
    if (found) {
      setRulesJson(JSON.stringify(found.rules, null, 2));
      setJsonError(null);
      if (!name.trim()) setName(found.label);
    }
  }

  async function handleCreate(event: React.FormEvent) {
    event.preventDefault();
    if (!name.trim()) return;

    // Parse before sending. A malformed rules object accepted by the browser becomes a
    // policy that matches nothing, which looks identical to a policy that is simply not
    // being applied — one of the hardest things to debug from the outside.
    let rules: Record<string, unknown>;
    try {
      rules = JSON.parse(rulesJson) as Record<string, unknown>;
    } catch {
      setJsonError("This is not valid JSON. Fix it before saving.");
      return;
    }

    setSaving(true);
    setJsonError(null);
    try {
      await api.createPolicy(name.trim(), rules);
      setName("");
      selectPreset(PRESETS[0].id);
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not save the policy.",
      );
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(policy: Policy) {
    const confirmed = window.confirm(
      `Delete the policy "${policy.name}"? Requests it was pinning will fall back to normal cost routing on the next request.`,
    );
    if (!confirmed) return;

    try {
      await api.deletePolicy(policy.id);
      await load();
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not delete the policy.",
      );
    }
  }

  const selected = PRESETS.find((entry) => entry.id === preset);

  return (
    <>
      <SectionHeader
        eyebrow="Governance"
        title="Routing policies"
        description="Rules that override the cost router. Policies are evaluated after an explicit per-request passthrough hint and before the complexity classifier, so a policy always beats the router but never beats a caller who explicitly asked for a specific model."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      <Card className="mb-8 p-5">
        <form onSubmit={handleCreate} className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label="Policy name"
              id="policy-name"
              value={name}
              onChange={setName}
              required
              placeholder="e.g. Protect production agents"
            />

            <div>
              <label
                htmlFor="preset"
                className="block text-sm font-medium text-[#403B35]"
              >
                Start from
              </label>
              <select
                id="preset"
                value={preset}
                onChange={(event) => selectPreset(event.target.value)}
                className="mt-1.5 w-full rounded-[12px] border border-[#C9B59C] bg-[#F9F8F6] px-3 py-2 text-sm text-[#0A0A0A]"
              >
                {PRESETS.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {selected && (
            <p className="rounded-xl border border-[#D9CFC7] bg-[#F9F8F6] px-3 py-2 text-xs font-medium leading-relaxed text-[#403B35]">
              {selected.description}
            </p>
          )}

          <div>
            <label
              htmlFor="rules"
              className="block text-sm font-medium text-[#403B35]"
            >
              Rules
            </label>
            <textarea
              id="rules"
              value={rulesJson}
              onChange={(event) => {
                setRulesJson(event.target.value);
                setJsonError(null);
              }}
              rows={7}
              spellCheck={false}
              aria-invalid={jsonError !== null}
              aria-describedby={jsonError ? "rules-error" : undefined}
              className={`mt-1.5 w-full rounded-[12px] border bg-[#F9F8F6] px-3 py-2 font-mono text-xs text-[#0A0A0A] ${
                jsonError ? "border-[#DC2626]" : "border-[#C9B59C]"
              }`}
            />
            {jsonError ? (
              <p id="rules-error" className="mt-1.5 text-xs font-bold text-[#DC2626]">
                {jsonError}
              </p>
            ) : (
              <p className="mt-1.5 text-xs font-medium text-[#70685E]">
                Edit freely — the presets are a starting point, not a limit.
              </p>
            )}
          </div>

          <div className="flex justify-end">
            <Button type="submit" disabled={saving || !name.trim()}>
              {saving ? "Saving…" : "Create policy"}
            </Button>
          </div>
        </form>
      </Card>

      {loading ? (
        <Card className="p-10 text-center text-xs font-bold text-[#70685E]">
          Loading policies…
        </Card>
      ) : policies.length === 0 ? (
        <EmptyState
          title="No policies yet"
          description="Without a policy, every request is routed purely on cost within its capability tier. Add one when you need a class of traffic held to a specific model or provider."
        />
      ) : (
        <div className="space-y-3">
          {policies.map((policy) => (
            <Card key={policy.id} className="p-5">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-black text-black">{policy.name}</h3>
                    <Badge tone={policy.is_active ? "accent" : "neutral"} size="sm">
                      {policy.is_active ? "active" : "inactive"}
                    </Badge>
                  </div>
                  <p className="mt-0.5 text-[11px] font-medium text-[#70685E]">
                    Created {formatRelative(policy.created_at)}
                  </p>
                </div>
                <Button variant="danger" onClick={() => handleDelete(policy)}>
                  Delete
                </Button>
              </div>

              <pre className="mt-3 overflow-x-auto rounded-xl border border-[#D9CFC7] bg-[#F9F8F6] p-3 font-mono text-[11px] leading-relaxed text-[#403B35]">
                {JSON.stringify(policy.rules, null, 2)}
              </pre>
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
