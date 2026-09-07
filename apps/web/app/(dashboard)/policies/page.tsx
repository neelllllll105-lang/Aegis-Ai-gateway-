"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type Policy, type PolicyRule } from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  Field,
  SectionHeader,
  UpgradeRequired,
} from "@/components/ui";
import { formatRelative } from "@/lib/format";
import { useAuth } from "@/lib/auth-context";

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

/**
 * The shapes the routing engine understands, as a human would describe them.
 *
 * The API expects `rules` as an array of `{"when": {...}, "then": {...}}` objects —
 * `engine::policy::RoutingPolicy::from_json` parses `Vec<Rule>` and falls back to an
 * empty (no-op) policy on any shape mismatch, and `routes::management::create_policy`
 * rejects that with a 400 before it can save silently-broken. These four presets used
 * to be flat objects with invented field names (`action`, `target_tier`, `pin_provider`,
 * `allowed_models`) that matched none of the real `Condition`/`Action` fields
 * (`complexity`, `model_requested`, `team`, `requires_tools`, `min_input_tokens` /
 * `model_tier`, `max_model_tier`, `pin_model`, `deny`, `passthrough`) — every save of
 * every preset failed the 400 check. Found while chasing an unrelated bug report.
 *
 * "Pin to a single provider" and "Restrict to an approved model list" are dropped rather
 * than patched: the policy DSL has no provider-level field and no negative/exclusion
 * matcher, so neither was ever expressible as a policy rule. A true model allowlist is a
 * real feature — it just lives on the API key itself (see Keys), not here.
 */
const PRESETS = [
  {
    id: "pin-complex",
    label: "Never downgrade complex work",
    description:
      "Requests classified complex always go to the model that was asked for. Simple and medium requests still route for cost.",
    rules: [{ when: { complexity: "complex" }, then: { passthrough: true } }],
  },
  {
    id: "force-economy",
    label: "Send simple requests to economy models",
    description:
      "Anything classified simple is served by the cheapest model in the economy tier. The largest single source of savings for most teams.",
    rules: [{ when: { complexity: "simple" }, then: { model_tier: "cheap" } }],
  },
  {
    id: "deny-model",
    label: "Block a specific model",
    description:
      "Requests naming this model are rejected outright rather than silently substituted onto something else. Useful for retiring a model or enforcing an exclusion.",
    rules: [{ when: { model_requested: "gpt-3.5-turbo" }, then: { deny: true } }],
  },
  {
    id: "pin-model",
    label: "Pin everything to one model",
    description:
      "Every request, regardless of what was asked for, is served by exactly this model. For a true per-key model allowlist instead, set it on the API key itself in Keys.",
    rules: [{ when: {}, then: { pin_model: "openai/gpt-4o-mini" } }],
  },
] as const;

export default function PoliciesPage() {
  const { canWrite, planFeatures } = useAuth();
  const hasPolicies = planFeatures.policies === true;
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
    if (!hasPolicies) {
      setLoading(false);
      return;
    }
    void load();
  }, [hasPolicies]);

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
    // being applied — one of the hardest things to debug from the outside. The API
    // requires an array of {"when": {...}, "then": {...}} objects — checked here too,
    // not just left to the server's 400, so the error shows up next to the field that's
    // actually wrong rather than as a bare request failure.
    let rules: PolicyRule[];
    try {
      const parsed: unknown = JSON.parse(rulesJson);
      if (!Array.isArray(parsed)) {
        setJsonError(
          'Rules must be an array: [{"when": {...}, "then": {...}}, ...].',
        );
        return;
      }
      rules = parsed as PolicyRule[];
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

  if (!hasPolicies) {
    return (
      <>
        <SectionHeader
          eyebrow="Governance"
          title="Routing policies"
          description="Rules that override the cost router, automatically, for the whole organization."
        />
        <UpgradeRequired feature="Routing policies" requiredPlan="Team" />
      </>
    );
  }

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

      {canWrite && (
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
                className="block text-sm font-medium text-[var(--color-muted)]"
              >
                Start from
              </label>
              <select
                id="preset"
                value={preset}
                onChange={(event) => selectPreset(event.target.value)}
                className="mt-1.5 w-full rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)]"
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
            <p className="rounded-xl border border-[var(--color-ink)] bg-[var(--color-surface2)] px-3 py-2 text-xs font-medium leading-relaxed text-[var(--color-muted)]">
              {selected.description}
            </p>
          )}

          <div>
            <label
              htmlFor="rules"
              className="block text-sm font-medium text-[var(--color-muted)]"
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
              className={`mt-1.5 w-full rounded-[12px] border bg-[var(--color-surface2)] px-3 py-2 font-mono text-xs text-[var(--color-ink)] ${
                jsonError ? "border-[var(--color-accent)]" : "border-[var(--color-accent)]"
              }`}
            />
            {jsonError ? (
              <p id="rules-error" className="mt-1.5 text-xs font-bold text-[var(--color-accent)]">
                {jsonError}
              </p>
            ) : (
              <p className="mt-1.5 text-xs font-medium text-[var(--color-muted-light)]">
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
      )}

      {loading ? (
        <Card className="p-10 text-center text-xs font-bold text-[var(--color-muted-light)]">
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
                    <h3 className="text-sm font-bold text-[var(--color-ink)]">{policy.name}</h3>
                    <Badge tone={policy.is_active ? "accent" : "neutral"} size="sm">
                      {policy.is_active ? "active" : "inactive"}
                    </Badge>
                  </div>
                  <p className="mt-0.5 text-[11px] font-medium text-[var(--color-muted-light)]">
                    Created {formatRelative(policy.created_at)}
                  </p>
                </div>
                {canWrite && (
                  <Button variant="danger" onClick={() => handleDelete(policy)}>
                    Delete
                  </Button>
                )}
              </div>

              <pre className="mt-3 overflow-x-auto rounded-xl border border-[var(--color-ink)] bg-[var(--color-surface2)] p-3 font-mono text-[11px] leading-relaxed text-[var(--color-muted)]">
                {JSON.stringify(policy.rules, null, 2)}
              </pre>
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
