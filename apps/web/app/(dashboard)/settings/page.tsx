"use client";

import { useEffect, useState } from "react";
import { api, ApiError, type BillingPlan, type OrgResponse } from "@/lib/api";
import { Badge, Card, ErrorState, SectionHeader } from "@/components/ui";
import { formatCount, formatUsd } from "@/lib/format";

/**
 * Organisation settings and plan.
 *
 * The privacy settings are read-only here on purpose: toggling zero-retention invalidates
 * the cache and is audit-logged, so it belongs behind a deliberate confirmation flow
 * rather than a switch someone can flip while scrolling past.
 */
export default function SettingsPage() {
  const [org, setOrg] = useState<OrgResponse | null>(null);
  const [plan, setPlan] = useState<BillingPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    Promise.all([api.org(), api.billingPlan()])
      .then(([orgResponse, planResponse]) => {
        if (cancelled) return;
        setOrg(orgResponse);
        setPlan(planResponse);
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

  if (loading) {
    return <p className="text-sm text-[var(--color-ink-subtle)]">Loading…</p>;
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
              <p className="mt-1 text-xs text-[var(--color-ink-subtle)]">
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
              <span className="text-[var(--color-ink-subtle)]">
                Month-to-date spend
              </span>
              <span className="tabular text-[var(--color-ink)]">
                {formatUsd(org?.usage.month_to_date_spend_mc ?? 0)}
              </span>
            </div>
            <div className="mt-2 flex items-baseline justify-between gap-4 text-sm">
              <span className="text-[var(--color-ink-subtle)]">
                Month-to-date savings
              </span>
              <span className="tabular text-[var(--color-accent)]">
                {formatUsd(org?.usage.month_to_date_savings_mc ?? 0)}
              </span>
            </div>
          </div>
        </Card>

        <Card className="p-6">
          <h3 className="text-sm font-medium text-[var(--color-ink)]">Data handling</h3>
          <p className="mt-1 text-xs text-[var(--color-ink-subtle)]">
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

          <p className="mt-5 rounded-[var(--radius)] border border-[var(--color-line)] bg-[var(--color-base)] p-3 text-xs leading-relaxed text-[var(--color-ink-subtle)]">
            Changing either setting invalidates your cache and is recorded in the audit
            log. Contact support to change them, or use{" "}
            <code className="text-[var(--color-ink-muted)]">PATCH /api/org</code>.
          </p>
        </Card>
      </div>
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
        <dt className="text-sm text-[var(--color-ink-subtle)]">{label}</dt>
        <dd
          className={`text-sm text-[var(--color-ink-muted)] ${mono ? "tabular" : ""}`}
        >
          {value}
        </dd>
      </div>
      {hint && (
        <p className="mt-1 text-xs leading-relaxed text-[var(--color-ink-faint)]">
          {hint}
        </p>
      )}
    </div>
  );
}
