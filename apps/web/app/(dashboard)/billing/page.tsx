"use client";

import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  type BillingPlan,
  type ChargebackReport,
  type CreditsResponse,
  type UsageSummaryResponse,
} from "@/lib/api";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ErrorState,
  SectionHeader,
  Stat,
  TableShell,
  Td,
  Th,
} from "@/components/ui";
import { formatCount, formatPercent, formatUsd } from "@/lib/format";

/**
 * Billing, credits and chargeback.
 *
 * The number that matters on this page is the customer net, not the fee. Aegis charges a
 * share of savings it created, so the honest framing is "you kept X after we took Y" —
 * and if that number is ever negative or zero the page should say so plainly rather than
 * showing a large gross-savings figure and hoping nobody subtracts.
 */
export default function BillingPage() {
  const [plan, setPlan] = useState<BillingPlan | null>(null);
  const [usage, setUsage] = useState<UsageSummaryResponse | null>(null);
  const [credits, setCredits] = useState<CreditsResponse | null>(null);
  const [chargeback, setChargeback] = useState<ChargebackReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [referralCode, setReferralCode] = useState("");
  const [claiming, setClaiming] = useState(false);
  const [claimNotice, setClaimNotice] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  async function load() {
    try {
      const [planResponse, usageResponse] = await Promise.all([
        api.billingPlan(),
        api.usageSummary(),
      ]);
      setPlan(planResponse);
      setUsage(usageResponse);
      setError(null);

      // Credits and chargeback are supplementary. A failure in either should not blank
      // out the plan and usage figures, which are the reason someone opened this page.
      const [creditResult, chargebackResult] = await Promise.allSettled([
        api.credits(),
        api.chargeback(),
      ]);
      if (creditResult.status === "fulfilled") setCredits(creditResult.value);
      if (chargebackResult.status === "fulfilled")
        setChargeback(chargebackResult.value);
    } catch (caught) {
      setError(
        caught instanceof ApiError ? caught.message : "Could not load billing.",
      );
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  async function handleClaim(event: React.FormEvent) {
    event.preventDefault();
    if (!referralCode.trim()) return;

    setClaiming(true);
    setClaimNotice(null);
    try {
      const result = await api.claimReferral(referralCode.trim());
      setClaimNotice(result.message);
      setReferralCode("");
      setCredits(await api.credits());
    } catch (caught) {
      setClaimNotice(
        caught instanceof ApiError ? caught.message : "Could not claim that code.",
      );
    } finally {
      setClaiming(false);
    }
  }

  async function copyReferral() {
    if (!credits) return;
    try {
      await navigator.clipboard.writeText(credits.referral_url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard access can be denied outright. Say nothing rather than throwing — the
      // URL is visible on screen and can be selected by hand.
    }
  }

  if (loading) {
    return (
      <>
        <SectionHeader eyebrow="Account" title="Billing" />
        <Card className="p-10 text-center text-xs font-bold text-[#70685E]">
          Loading billing…
        </Card>
      </>
    );
  }

  const summary = usage?.summary;
  const netKept = usage?.derived.customer_net_mc ?? 0;

  return (
    <>
      <SectionHeader
        eyebrow="Account"
        title="Billing and credits"
        description="Aegis charges a share of the savings it produced, and nothing when it produced none. Provider costs are billed to you directly by each provider under BYOK — they never pass through us."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      {plan && (
        <Card className="mb-6 p-5">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-lg font-black capitalize text-black">
                  {plan.plan} plan
                </h3>
                <Badge tone="accent" size="sm">
                  {plan.savings_share_percent}% of savings
                </Badge>
              </div>
              <p className="mt-1 text-xs font-medium leading-relaxed text-[#403B35]">
                {plan.subscription_mc > 0
                  ? `${formatUsd(plan.subscription_mc)} per month, plus ${plan.savings_share_percent}% of verified savings.`
                  : `No subscription fee. You pay ${plan.savings_share_percent}% of verified savings and nothing else.`}
              </p>
            </div>
            <dl className="flex gap-8">
              <div>
                <dt className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                  Rate limit
                </dt>
                <dd className="font-mono text-sm font-bold text-black">
                  {formatCount(plan.limits.requests_per_minute)}/min
                </dd>
              </div>
              <div>
                <dt className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                  Monthly allowance
                </dt>
                <dd className="font-mono text-sm font-bold text-black">
                  {plan.limits.monthly_request_allowance === null
                    ? "unlimited"
                    : formatCount(plan.limits.monthly_request_allowance)}
                </dd>
              </div>
              <div>
                <dt className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                  BYOK
                </dt>
                <dd className="font-mono text-sm font-bold text-black">
                  {plan.limits.byok ? "included" : "not on this plan"}
                </dd>
              </div>
            </dl>
          </div>
        </Card>
      )}

      {summary && (
        <div className="mb-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <Stat
            label="Would have cost"
            value={formatUsd(summary.baseline_cost_mc)}
            sublabel="At the model you asked for"
          />
          <Stat
            label="Actually cost"
            value={formatUsd(summary.actual_cost_mc)}
            sublabel="Billed by providers"
          />
          <Stat
            label="Aegis fee"
            value={formatUsd(summary.aegis_fee_mc)}
            sublabel={`${plan?.savings_share_percent ?? 0}% of gross savings`}
          />
          <Stat
            label="You kept"
            value={formatUsd(netKept)}
            sublabel={`${formatPercent(usage.derived.savings_percent)} saved after our fee`}
            accent
          />
        </div>
      )}

      {credits && (
        <Card className="mb-8 p-5">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="min-w-0">
              <h3 className="text-sm font-black text-black">Referral credit</h3>
              <p className="mt-1 max-w-xl text-xs font-medium leading-relaxed text-[#403B35]">
                {credits.terms} Each side receives{" "}
                {formatUsd(credits.credit_per_referral_mc)}, applied against future Aegis
                fees.
              </p>
            </div>
            <div className="text-right">
              <div className="text-[10px] font-black uppercase tracking-wider text-[#70685E]">
                Available credit
              </div>
              <div className="font-mono text-2xl font-black text-black">
                {formatUsd(credits.balance_mc)}
              </div>
            </div>
          </div>

          <div className="mt-4 grid gap-4 sm:grid-cols-2">
            <div>
              <div className="mb-1.5 text-xs font-bold text-[#403B35]">
                Your referral link
              </div>
              <div className="flex gap-2">
                <code className="min-w-0 flex-1 truncate rounded-xl border border-[#D9CFC7] bg-[#F9F8F6] px-3 py-2 font-mono text-[11px] text-[#403B35]">
                  {credits.referral_url}
                </code>
                <Button variant="secondary" onClick={copyReferral}>
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
            </div>

            <form onSubmit={handleClaim}>
              <label
                htmlFor="referral-code"
                className="mb-1.5 block text-xs font-bold text-[#403B35]"
              >
                Have a code?
              </label>
              <div className="flex gap-2">
                <input
                  id="referral-code"
                  value={referralCode}
                  onChange={(event) => setReferralCode(event.target.value)}
                  placeholder="their-org-slug"
                  className="min-w-0 flex-1 rounded-xl border border-[#C9B59C] bg-[#F9F8F6] px-3 py-2 font-mono text-[11px] text-[#0A0A0A]"
                />
                <Button
                  type="submit"
                  variant="secondary"
                  disabled={claiming || !referralCode.trim()}
                >
                  {claiming ? "Claiming…" : "Claim"}
                </Button>
              </div>
              {claimNotice && (
                <p className="mt-1.5 text-[11px] font-bold text-[#403B35]">
                  {claimNotice}
                </p>
              )}
            </form>
          </div>
        </Card>
      )}

      <div className="mb-3 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h3 className="text-sm font-black text-black">Chargeback by cost center</h3>
          <p className="mt-0.5 text-xs font-medium text-[#70685E]">
            Spend attributed to teams, for internal recharge. Export as CSV for a finance
            system.
          </p>
        </div>
        <a
          href={api.chargebackCsvUrl()}
          className="inline-flex items-center gap-2 rounded-xl border border-[#D9CFC7] bg-white px-5 py-2.5 text-xs font-bold text-black shadow-xs transition-all hover:bg-[#EFE9E3]"
        >
          Download CSV
        </a>
      </div>

      {!chargeback || chargeback.lines.length === 0 ? (
        <EmptyState
          title="Nothing to attribute yet"
          description="Spend is attributed through the team an API key belongs to. Create teams and assign keys to them to get a breakdown here."
        />
      ) : (
        <>
          <TableShell>
            <thead>
              <tr>
                <Th>Cost center</Th>
                <Th align="right">Requests</Th>
                <Th align="right">Spend</Th>
                <Th align="right">Saved</Th>
                <Th align="right">Aegis fee</Th>
                <Th align="right">Share</Th>
              </tr>
            </thead>
            <tbody>
              {chargeback.lines.map((line) => (
                <tr key={line.cost_center}>
                  <Td>{line.cost_center}</Td>
                  <Td align="right" mono>
                    {formatCount(line.requests)}
                  </Td>
                  <Td align="right" mono>
                    {formatUsd(line.spend_mc)}
                  </Td>
                  <Td align="right" mono>
                    {formatUsd(line.savings_mc)}
                  </Td>
                  <Td align="right" mono muted>
                    {formatUsd(line.fee_mc)}
                  </Td>
                  <Td align="right" mono muted>
                    {line.share_percent.toFixed(1)}%
                  </Td>
                </tr>
              ))}
            </tbody>
          </TableShell>

          {chargeback.unattributed_mc > 0 && (
            <p className="mt-3 text-[11px] font-medium leading-relaxed text-[#70685E]">
              {formatUsd(chargeback.unattributed_mc)} of spend could not be attributed to
              a cost center, because the API keys behind it are not assigned to a team. It
              is reported separately rather than spread across the lines above.
            </p>
          )}
        </>
      )}
    </>
  );
}
