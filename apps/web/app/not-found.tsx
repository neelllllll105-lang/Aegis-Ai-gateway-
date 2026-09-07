import type { Metadata } from "next";
import Link from "next/link";
import { AegisLogo } from "@/components/ui";

export const metadata: Metadata = {
  title: "Page Not Found",
  robots: { index: false, follow: false },
};

/**
 * Global 404 — catches any URL that matches no route at all.
 *
 * Deliberately self-contained rather than reusing the marketing header: a mistyped or
 * stale link could point anywhere (a dashboard page, an old docs anchor, a typo), so this
 * makes no assumption about which part of the site the visitor meant to reach. One clear
 * way back is more useful here than a full nav bar that may not even apply.
 */
export default function NotFound() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-4 py-16 text-center text-[var(--color-paper-on-desk)]">
      <Link href="/" className="mb-8 flex items-center gap-2.5 text-lg font-bold tracking-tight group">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface2)] text-[var(--color-ink)] transition-transform group-hover:scale-105">
          <AegisLogo className="h-5 w-5 text-[var(--color-accent)]" />
        </div>
        <span className="font-serif text-xl font-semibold">Aegis</span>
      </Link>

      <p className="font-mono text-xs font-bold uppercase tracking-[0.2em] text-[var(--color-muted-on-desk)]">
        Error 404
      </p>
      <h1 className="mt-3 font-serif text-6xl font-semibold tracking-tight text-[var(--color-paper-on-desk)] sm:text-7xl">
        This page took a wrong turn.
      </h1>
      <p className="mx-auto mt-4 max-w-md text-sm font-medium text-[var(--color-muted-on-desk)]">
        Nothing lives at this address — the link may be old, or the URL has a typo.
        Everything else on Aegis is exactly where you left it.
      </p>

      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <Link
          href="/"
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-ink)] px-5 py-2.5 text-xs font-bold text-[var(--color-surface)] shadow-[3px_3px_0_var(--shadow-color)] transition-all duration-150 hover:-translate-x-px hover:-translate-y-px hover:shadow-[4px_4px_0_var(--shadow-color)] active:translate-x-px active:translate-y-px active:shadow-[1px_1px_0_var(--shadow-color)]"
        >
          Back to Aegis
        </Link>
        <Link
          href="/dashboard"
          className="inline-flex items-center justify-center gap-2 rounded-xl border-[1.5px] border-[var(--color-desk-line)] bg-transparent px-5 py-2.5 text-xs font-bold text-[var(--color-paper-on-desk)] transition-colors hover:bg-[var(--color-desk-raised)]"
        >
          Go to dashboard
        </Link>
      </div>
    </div>
  );
}
