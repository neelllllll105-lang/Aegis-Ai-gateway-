import type { Metadata } from "next";
import Link from "next/link";
import { EmptyState } from "@/components/ui";

export const metadata: Metadata = {
  title: "Page Not Found",
};

/**
 * 404 scoped to the dashboard route group. Renders inside `(dashboard)/layout.tsx`, so the
 * sidebar and org switcher stay put — a signed-in person following a stale link should not
 * lose their place in the product to find out the page they wanted doesn't exist.
 */
export default function DashboardNotFound() {
  return (
    <EmptyState
      title="This page doesn't exist"
      description="The link may be old, or the URL has a typo. Everything else in your dashboard is right where you left it."
      action={
        <Link
          href="/dashboard"
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-ink)] px-5 py-2.5 text-xs font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all duration-150 hover:-translate-x-px hover:-translate-y-px hover:shadow-[4px_4px_0_var(--shadow-color)]"
        >
          Back to overview
        </Link>
      }
    />
  );
}
