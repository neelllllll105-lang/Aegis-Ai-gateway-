"use client";

import { useState } from "react";
import { Button, Card } from "@/components/ui";
import { useAuth } from "@/lib/auth-context";
import type { PlanFeature } from "@/lib/api";

interface Step {
  title: string;
  description: string;
  /** Omitted for a step every Team/Enterprise admin sees. Present skips the step when the
   *  org's plan doesn't include it — today this only ever excludes the Enterprise step for
   *  a Team-plan viewer, since every other feature here is already included by Team. */
  feature?: PlanFeature;
}

const STEPS: Step[] = [
  {
    title: "Welcome to Aegis",
    description:
      "A quick tour of what's here as an owner or admin — five stops, about a minute. You can replay this anytime from Settings.",
  },
  {
    title: "Overview, Usage & Request Logs",
    description:
      "See exactly where every dollar and token went — per model, per project, per request. Nothing here is estimated after the fact; it's the same figures the provider billed you for.",
  },
  {
    title: "API Keys & Providers",
    description:
      "Issue a key per person or per project so spend is attributed automatically. Connect your own provider accounts here too, for full control over which credentials handle which traffic.",
    feature: "byok",
  },
  {
    title: "Budgets & Policies",
    description:
      "Cap spend per project, per person, or org-wide — hard or soft. Policies let you pin models, set a routing mode, or deny a request outright, automatically, for the whole organization.",
    feature: "budgets",
  },
  {
    title: "People & Teams",
    description:
      "Organize the organization into projects, assign a lead to each, and see every project's spend broken out on its own — rename a project anytime and its history follows the new name.",
    feature: "team_management",
  },
  {
    title: "Savings & ROI",
    description:
      "A live breakdown of what Aegis is actually saving you, split by routing, caching, and compression — so the number is never just a single unexplained percentage.",
    feature: "savings",
  },
  {
    title: "Enterprise controls",
    description:
      "Settings includes SSO/SCIM configuration, an exportable audit log, and data-residency controls — everything a compliance review will ask for.",
    feature: "enterprise",
  },
];

/**
 * A short, sequential walkthrough for a Team/Enterprise owner or admin's first login —
 * see `DashboardShell`'s trigger condition. No tour library is installed in this app, so
 * this is a small hand-built sequence reusing `Card`/`Button` rather than introducing a
 * dependency with its own visual language.
 */
export function OnboardingTour({ onFinish }: { onFinish: () => void }) {
  const { planFeatures } = useAuth();
  const steps = STEPS.filter((step) => !step.feature || planFeatures[step.feature]);
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const isLast = index === steps.length - 1;

  // Every unconditional step (no `feature`) always survives the filter, so this is
  // unreachable in practice — a defensive bail rather than a non-null assertion, since a
  // future edit that made every step conditional should fail safe, not throw.
  if (!step) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Dashboard walkthrough"
      className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--color-ink)]/50 px-4"
    >
      <Card className="w-full max-w-md p-6">
        <div className="mb-4 flex items-center gap-1.5">
          {steps.map((_, i) => (
            <span
              key={i}
              className={`h-1.5 flex-1 rounded-full ${
                i <= index ? "bg-[var(--color-accent)]" : "bg-[var(--color-surface2)]"
              }`}
            />
          ))}
        </div>

        <div className="font-mono text-[10px] font-bold uppercase tracking-[0.18em] text-[var(--color-muted-light)]">
          Step {index + 1} of {steps.length}
        </div>
        <h2 className="mt-1.5 font-serif text-lg font-semibold text-[var(--color-ink)]">
          {step.title}
        </h2>
        <p className="mt-2 text-xs leading-relaxed text-[var(--color-muted)] font-medium">
          {step.description}
        </p>

        <div className="mt-6 flex items-center justify-between gap-3">
          <Button variant="ghost" onClick={onFinish}>
            Skip
          </Button>
          <div className="flex gap-2">
            {index > 0 && (
              <Button variant="secondary" onClick={() => setIndex((i) => i - 1)}>
                Back
              </Button>
            )}
            <Button
              variant="primary"
              onClick={() => (isLast ? onFinish() : setIndex((i) => i + 1))}
            >
              {isLast ? "Done" : "Next"}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}
