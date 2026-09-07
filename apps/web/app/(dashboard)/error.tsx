"use client";

import { useEffect } from "react";
import Link from "next/link";
import { Card } from "@/components/ui";

/**
 * Error boundary scoped to the dashboard route group — a crash on, say, the usage page
 * loses that one page's content but keeps the sidebar and org context intact, rather than
 * dropping a signed-in person back to a bare error screen with no way back into the
 * product. Shows no exception detail, same reasoning as the root `error.tsx`.
 */
export default function DashboardError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error(error);
  }, [error]);

  return (
    <Card className="px-6 py-14 text-center">
      <div className="mx-auto mb-3 flex h-10 w-10 items-center justify-center rounded-full border border-[var(--color-accent)] bg-[var(--color-accent-bg)] text-[var(--color-accent)]">
        <svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor">
          <path
            fillRule="evenodd"
            d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z"
            clipRule="evenodd"
          />
        </svg>
      </div>
      <p className="font-serif text-sm font-semibold text-[var(--color-ink)]">
        This page hit a snag
      </p>
      <p className="mx-auto mt-1 max-w-md text-xs font-medium text-[var(--color-muted)]">
        Nothing you did caused this, and the rest of your dashboard is unaffected. Try again
        — if it keeps happening, our team can look it up
        {error.digest ? " from the reference below" : ""}.
      </p>
      {error.digest && (
        <p className="mt-2 font-mono text-[11px] text-[var(--color-muted-light)]">
          Reference: {error.digest}
        </p>
      )}
      <div className="mt-5 flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={reset}
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-ink)] px-5 py-2.5 text-xs font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all duration-150 hover:-translate-x-px hover:-translate-y-px hover:shadow-[4px_4px_0_var(--shadow-color)]"
        >
          Try again
        </button>
        <Link
          href="/dashboard"
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] px-5 py-2.5 text-xs font-bold text-[var(--color-ink)] transition-colors hover:bg-[var(--color-surface2)]"
        >
          Back to overview
        </Link>
      </div>
    </Card>
  );
}
