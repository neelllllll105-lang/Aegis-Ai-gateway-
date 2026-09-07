"use client";

import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  ROUTING_MODES,
  type BillingPlan,
  type OrgResponse,
  type RoutingMode,
} from "@/lib/api";
import { Badge, Button, Card, ErrorState, SectionHeader } from "@/components/ui";
import { formatCount, formatUsd } from "@/lib/format";
import { useAuth } from "@/lib/auth-context";
import { OnboardingTour } from "@/components/onboarding-tour";

const ROUTING_MODE_LABEL: Record<RoutingMode, string> = {
  auto: "Auto",
  quality: "Quality",
  balanced: "Balanced",
  economy: "Economy",
  passthrough: "Passthrough",
};

/**
 * Organisation settings and plan.
 *
 * The privacy settings are read-only here on purpose: toggling zero-retention invalidates
 * the cache and is audit-logged, so it belongs behind a deliberate confirmation flow
 * rather than a switch someone can flip while scrolling past.
 */
export default function SettingsPage() {
  const { canWrite, role } = useAuth();
  const [org, setOrg] = useState<OrgResponse | null>(null);
  const [plan, setPlan] = useState<BillingPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [replayingTour, setReplayingTour] = useState(false);

  const [routingMode, setRoutingMode] = useState<RoutingMode | "">("");
  const [savingMode, setSavingMode] = useState(false);
  const [modeNotice, setModeNotice] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    Promise.all([api.org(), api.billingPlan()])
      .then(([orgResponse, planResponse]) => {
        if (cancelled) return;
        setOrg(orgResponse);
        setPlan(planResponse);
        setRoutingMode(orgResponse.organization.default_routing_mode ?? "");
        setLoading(false);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setError(
          caught instanceof ApiError ? caught.message : "Could not load settings.",
        );
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  async function handleSaveRoutingMode() {
    if (!routingMode) return;
    setSavingMode(true);
    setModeNotice(null);
    try {
      const updated = await api.updateOrg({ default_routing_mode: routingMode });
      setOrg((current) =>
        current ? { ...current, organization: updated } : current,
      );
      setModeNotice("Saved.");
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not save the routing default.",
      );
    } finally {
      setSavingMode(false);
    }
  }

  if (loading) {
    return <p className="text-sm text-[var(--color-muted-on-desk)]">Loading…</p>;
  }
  if (error) {
    return <ErrorState message={error} />;
  }

  const organization = org?.organization;

  return (
    <>
      <SectionHeader title="Settings" description="Organisation, plan, and data handling." />

      <div className="space-y-6">
        <Card className="p-6">
          <h3 className="text-sm font-medium text-[var(--color-ink)]">Organisation</h3>
          <dl className="mt-4 space-y-3">
            <SettingRow label="Name" value={organization?.name ?? "—"} />
            <SettingRow label="Slug" value={organization?.slug ?? "—"} mono />
            <SettingRow label="Region" value={organization?.region ?? "—"} mono />
            <SettingRow
              label="Billing email"
              value={organization?.billing_email ?? "not set"}
            />
          </dl>
        </Card>

        <Card className="p-6">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h3 className="text-sm font-medium text-[var(--color-ink)]">Plan</h3>
              <p className="mt-1 text-xs text-[var(--color-muted-light)]">
                We charge a share of savings only when there are savings to share.
              </p>
            </div>
            <Badge tone="accent">{plan?.plan ?? "free"}</Badge>
          </div>

          <dl className="mt-4 space-y-3">
            <SettingRow
              label="Subscription"
              value={
                plan && plan.subscription_mc > 0
                  ? `${formatUsd(plan.subscription_mc)}/month`
                  : "Free"
              }
            />
            <SettingRow
              label="Savings share"
              value={`${plan?.savings_share_percent.toFixed(0) ?? 0}% of verified savings`}
            />
            <SettingRow
              label="Rate limit"
              value={`${formatCount(plan?.limits.requests_per_minute ?? 0)} requests/minute`}
            />
            <SettingRow
              label="Monthly allowance"
              value={
                plan?.limits.monthly_request_allowance
                  ? `${formatCount(plan.limits.monthly_request_allowance)} requests`
                  : "Unlimited"
              }
            />
            <SettingRow
              label="Bring your own keys"
              value={plan?.limits.byok ? "Enabled" : "Upgrade required"}
            />
          </dl>

          <div className="mt-5 border-t border-[var(--color-line)] pt-4">
            <div className="flex items-baseline justify-between gap-4 text-sm">
              <span className="text-[var(--color-muted-light)]">
                Month-to-date spend
              </span>
              <span className="tabular text-[var(--color-ink)]">
                {formatUsd(org?.usage.month_to_date_spend_mc ?? 0)}
              </span>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-4 text-sm">
              <span className="text-[var(--color-muted-light)]">
                Month-to-date savings
              </span>
              <span className="tabular text-[var(--color-accent)]">
                {formatUsd(org?.usage.month_to_date_savings_mc ?? 0)}
              </span>
            </div>
          </div>
        </Card>

        {(role === "owner" || role === "admin") &&
          (plan?.plan === "team" || plan?.plan === "enterprise") && (
            <Card className="p-6">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <h3 className="text-sm font-medium text-[var(--color-ink)]">
                    Walkthrough
                  </h3>
                  <p className="mt-1 text-xs text-[var(--color-muted-light)]">
                    The tour shown automatically the first time an owner or admin signs in.
                  </p>
                </div>
                <Button variant="secondary" onClick={() => setReplayingTour(true)}>
                  Replay walkthrough
                </Button>
              </div>
            </Card>
          )}

        <Card className="p-6">
          <h3 className="text-sm font-medium text-[var(--color-ink)]">Routing</h3>
          <p className="mt-1 text-xs text-[var(--color-muted-light)]">
            The mode a request uses when the caller sends no{" "}
            <code className="text-[var(--color-muted)]">X-Aegis-Routing-Hint</code> header
            and its key or project sets no default of its own — the last rung before the
            ladder&rsquo;s own <code className="text-[var(--color-muted)]">auto</code>.
          </p>

          <div className="mt-4 flex flex-wrap items-end gap-3">
            <div>
              <label
                htmlFor="org-routing-mode"
                className="block text-sm font-medium text-[var(--color-muted)]"
              >
                Organisation default
              </label>
              <select
                id="org-routing-mode"
                value={routingMode}
                disabled={!canWrite}
                onChange={(event) => {
                  setRoutingMode(event.target.value as RoutingMode);
                  setModeNotice(null);
                }}
                className="mt-1.5 w-56 rounded-[12px] border border-[var(--color-accent)] bg-[var(--color-surface2)] px-3 py-2 text-sm text-[var(--color-ink)] disabled:opacity-50"
              >
                {ROUTING_MODES.map((mode) => (
                  <option key={mode} value={mode}>
                    {ROUTING_MODE_LABEL[mode]}
                  </option>
                ))}
              </select>
            </div>
            {canWrite && (
              <Button
                onClick={handleSaveRoutingMode}
                disabled={
                  savingMode || routingMode === (organization?.default_routing_mode ?? "auto")
                }
              >
                {savingMode ? "Saving…" : "Save"}
              </Button>
            )}
            {modeNotice && (
              <span className="text-xs font-bold text-[var(--color-positive)]">
                {modeNotice}
              </span>
            )}
          </div>
        </Card>

        <Card className="p-6">
          <h3 className="text-sm font-medium text-[var(--color-ink)]">Data handling</h3>
          <p className="mt-1 text-xs text-[var(--color-muted-light)]">
            Prompt and response content is never stored unless you explicitly opt in.
          </p>

          <dl className="mt-4 space-y-3">
            <SettingRow
              label="Zero retention"
              value={organization?.zero_retention ? "Enabled" : "Disabled"}
              hint={
                organization?.zero_retention
                  ? "Caching is disabled and nothing is written. Savings come from routing alone."
                  : "Responses are cached to your organisation only. Cache keys are scoped so cross-tenant hits are impossible."
              }
            />
            <SettingRow
              label="Content capture"
              value={organization?.content_capture ? "Enabled" : "Disabled"}
              hint={
                organization?.content_capture
                  ? "Prompts and responses are stored encrypted with a key derived for this organisation."
                  : "Only operational metadata is stored: model, tokens, cost, latency, cache status."
              }
            />
          </dl>

          <p className="mt-5 rounded-[var(--radius)] border border-[var(--color-line)] bg-[var(--color-surface2)] p-3 text-xs leading-relaxed text-[var(--color-muted-light)]">
            Changing either setting invalidates your cache and is recorded in the audit
            log. Contact support to change them, or use{" "}
            <code className="text-[var(--color-muted)]">PATCH /api/org</code>.
          </p>
        </Card>
      </div>

      {replayingTour && (
        <OnboardingTour onFinish={() => setReplayingTour(false)} />
      )}
    </>
  );
}

function SettingRow({
  label,
  value,
  hint,
  mono = false,
}: {
  label: string;
  value: string;
  hint?: string;
  mono?: boolean;
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between gap-4">
        <dt className="text-sm text-[var(--color-muted-light)]">{label}</dt>
        <dd
          className={`text-sm text-[var(--color-muted)] ${mono ? "tabular" : ""}`}
        >
          {value}
        </dd>
      </div>
      {hint && (
        <p className="mt-1 text-xs leading-relaxed text-[var(--color-muted-light)]">
          {hint}
        </p>
      )}
    </div>
  );
}
