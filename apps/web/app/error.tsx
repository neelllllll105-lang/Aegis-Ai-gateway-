"use client";

import { useEffect } from "react";
import Link from "next/link";
import { AegisLogo } from "@/components/ui";

/**
 * Global error boundary — catches a render-time exception anywhere under the root layout.
 * Must be a Client Component; Next.js requirement, not a stylistic choice.
 *
 * Deliberately shows nothing from `error.message`: this boundary can catch failures from
 * any page, including ones handling billing or credentials, and an unfiltered exception
 * message is exactly the kind of internal detail `AegisError::Internal`'s own client-facing
 * rule on the gateway side already refuses to leak. `error.digest` is safe and useful —
 * it's an opaque correlation id Next.js mints for production errors, meant to be quoted to
 * support, not a description of what broke.
 */
export default function GlobalErrorBoundary({
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
    <div className="flex min-h-screen flex-col items-center justify-center px-4 py-16 text-center text-[var(--color-paper-on-desk)]">
      <Link href="/" className="mb-8 flex items-center gap-2.5 text-lg font-bold tracking-tight group">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface2)] text-[var(--color-ink)] transition-transform group-hover:scale-105">
          <AegisLogo className="h-5 w-5 text-[var(--color-accent)]" />
        </div>
        <span className="font-serif text-xl font-semibold">Aegis</span>
      </Link>

      <p className="font-mono text-xs font-bold uppercase tracking-[0.2em] text-[var(--color-accent)]">
        Something went wrong
      </p>
      <h1 className="mt-3 max-w-lg font-serif text-4xl font-semibold tracking-tight text-[var(--color-paper-on-desk)] sm:text-5xl">
        This page hit a snag on our end.
      </h1>
      <p className="mx-auto mt-4 max-w-md text-sm font-medium text-[var(--color-muted-on-desk)]">
        Nothing you did caused this. Try again — if it keeps happening, our team can look
        it up{error.digest ? " from the reference below" : ""}.
      </p>

      {error.digest && (
        <p className="mt-3 font-mono text-[11px] text-[var(--color-muted-on-desk)]">
          Reference: <span className="text-[var(--color-paper-on-desk)]">{error.digest}</span>
        </p>
      )}

      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={reset}
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-ink)] px-5 py-2.5 text-xs font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all duration-150 hover:-translate-x-px hover:-translate-y-px hover:shadow-[4px_4px_0_var(--shadow-color)] active:translate-x-px active:translate-y-px active:shadow-[1px_1px_0_var(--shadow-color)]"
        >
          Try again
        </button>
        <Link
          href="/"
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-desk-line)] bg-transparent px-5 py-2.5 text-xs font-bold text-[var(--color-paper-on-desk)] transition-colors hover:bg-[var(--color-desk-raised)]"
        >
          Back to Aegis
        </Link>
      </div>
    </div>
  );
}
