"use client";

import { useEffect, useMemo, useState } from "react";
import { api, ApiError, type ModelPrice } from "@/lib/api";
import {
  Badge,
  Card,
  ErrorState,
  SectionHeader,
  TableShell,
  Td,
  Th,
} from "@/components/ui";

const TIERS = ["all", "economy", "standard", "premium"] as const;
type Tier = (typeof TIERS)[number];

/**
 * The model catalogue, framed as a substitution question.
 *
 * A price list on its own answers nothing useful — nobody memorises dollars per million
 * tokens. What a customer actually wants to know is: if the router moves off this model,
 * what does it move to, and what does that save? So every row carries the cheapest
 * equivalent in its own tier and the resulting percentage, which is precisely the
 * comparison the router makes.
 */
export default function ModelsPage() {
  const [models, setModels] = useState<ModelPrice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [tier, setTier] = useState<Tier>("all");
  const [showRetired, setShowRetired] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .models()
      .then((response) => {
        if (!cancelled) setModels(response.models);
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setError(
            caught instanceof ApiError
              ? caught.message
              : "Could not load the model catalogue.",
          );
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const visible = useMemo(
    () =>
      models.filter(
        (model) =>
          (tier === "all" || model.tier === tier) &&
          (showRetired || model.is_active),
      ),
    [models, tier, showRetired],
  );

  const retiredCount = models.filter((model) => !model.is_active).length;

  return (
    <>
      <SectionHeader
        eyebrow="Catalogue"
        title="Models and pricing"
        description="Every model Aegis can route to, priced per million tokens at a 3:1 input-to-output blend. The saving column is what the router would gain by substituting the cheapest model in the same capability tier."
      />

      {error && (
        <div className="mb-6">
          <ErrorState message={error} />
        </div>
      )}

      <Card className="mb-6 p-4">
        <div className="flex flex-wrap items-center gap-4">
          <div
            className="flex gap-1 rounded-xl border border-[#D9CFC7] bg-[#F9F8F6] p-1"
            role="group"
            aria-label="Filter by tier"
          >
            {TIERS.map((option) => (
              <button
                key={option}
                type="button"
                onClick={() => setTier(option)}
                aria-pressed={tier === option}
                className={`rounded-lg px-3 py-1.5 text-xs font-bold capitalize transition-colors ${
                  tier === option
                    ? "border border-[#D9CFC7] bg-white text-black shadow-xs"
                    : "text-[#403B35] hover:text-black"
                }`}
              >
                {option}
              </button>
            ))}
          </div>

          {retiredCount > 0 && (
            <label className="flex items-center gap-2 text-xs font-bold text-[#403B35]">
              <input
                type="checkbox"
                checked={showRetired}
                onChange={(event) => setShowRetired(event.target.checked)}
                className="h-3.5 w-3.5 accent-[#C9B59C]"
              />
              Show {retiredCount} retired model{retiredCount === 1 ? "" : "s"}
            </label>
          )}

          <span className="ml-auto font-mono text-[11px] font-bold text-[#70685E]">
            {visible.length} shown
          </span>
        </div>
      </Card>

      {loading ? (
        <Card className="p-10 text-center text-xs font-bold text-[#70685E]">
          Loading catalogue…
        </Card>
      ) : (
        <TableShell>
          <thead>
            <tr>
              <Th>Model</Th>
              <Th>Provider</Th>
              <Th>Tier</Th>
              <Th align="right">Input /Mtok</Th>
              <Th align="right">Output /Mtok</Th>
              <Th align="right">Blended</Th>
              <Th align="right">Cheapest in tier</Th>
              <Th align="right">Saving</Th>
              <Th align="right">Context</Th>
            </tr>
          </thead>
          <tbody>
            {visible.map((model) => (
              <tr key={`${model.provider}:${model.model_id}`}>
                <Td>
                  <div className="flex items-center gap-2">
                    <span className="font-mono font-bold">{model.model_id}</span>
                    {!model.is_active && (
                      <Badge tone="warn" size="sm">
                        retired
                      </Badge>
                    )}
                  </div>
                  <div className="mt-0.5 flex gap-1.5">
                    {model.supports_tools && (
                      <span className="text-[10px] font-bold text-[#70685E]">
                        tools
                      </span>
                    )}
                    {model.supports_vision && (
                      <span className="text-[10px] font-bold text-[#70685E]">
                        vision
                      </span>
                    )}
                  </div>
                </Td>
                <Td muted>{model.provider}</Td>
                <Td>
                  <Badge
                    tone={model.tier === "premium" ? "accent" : "neutral"}
                    size="sm"
                  >
                    {model.tier}
                  </Badge>
                </Td>
                <Td align="right" mono>
                  {perMtok(model.input_per_mtok_mc)}
                </Td>
                <Td align="right" mono>
                  {perMtok(model.output_per_mtok_mc)}
                </Td>
                <Td align="right" mono>
                  {perMtok(model.blended_per_mtok_mc)}
                </Td>
                <Td align="right" mono muted>
                  {perMtok(model.cheapest_in_tier_mc)}
                </Td>
                <Td align="right" mono>
                  {model.potential_saving_pct > 0 ? (
                    <span className="font-black text-[var(--color-positive)]">
                      &minus;{model.potential_saving_pct}%
                    </span>
                  ) : (
                    <span className="text-[#70685E]">cheapest</span>
                  )}
                </Td>
                <Td align="right" mono muted>
                  {formatContext(model.context_window)}
                </Td>
              </tr>
            ))}
          </tbody>
        </TableShell>
      )}

      <p className="mt-4 text-[11px] font-medium leading-relaxed text-[#70685E]">
        Prices are list prices published by each provider, carried with their source date
        in the pricing table. Aegis never marks them up: under BYOK you are billed by the
        provider directly, and Aegis charges only a share of verified savings.
      </p>
    </>
  );
}

/** Micro-cents per million tokens, rendered as dollars. */
function perMtok(microCents: number): string {
  const dollars = microCents / 1_000_000;
  if (dollars === 0) return "$0";
  return dollars < 1 ? `$${dollars.toFixed(3)}` : `$${dollars.toFixed(2)}`;
}

function formatContext(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}K`;
  return String(tokens);
}
