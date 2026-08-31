import Link from "next/link";
import { AegisLogo } from "@/components/ui";

/**
 * Aegis Marketing Shell — dark desk chrome (header, footer), warm paper
 * content sections below. See app/globals.css and docs/design.md.
 */
export default function MarketingLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="mx-auto max-w-7xl px-4 py-6 sm:px-6 sm:py-10 text-[var(--color-paper-on-desk)]">
      {/* Header with Aegis Sleek Minimalist Logo */}
      <header className="mb-8 border-b border-[var(--color-desk-line)] pb-4">
        <div className="relative flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
          <Link
            href="/"
            className="flex items-center gap-2.5 text-lg font-bold tracking-tight text-[var(--color-paper-on-desk)] group"
          >
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-[var(--color-surface2)] border-[1.5px] border-[var(--color-ink)] text-[var(--color-ink)] transition-transform group-hover:scale-105">
              <AegisLogo className="w-5 h-5 text-[var(--color-accent)]" />
            </div>
            <span className="font-serif text-[var(--color-paper-on-desk)] font-semibold text-xl tracking-tight">Aegis</span>
            <span className="hidden sm:inline-block text-[10px] uppercase font-bold tracking-wider text-[var(--color-muted-on-desk)] bg-[var(--color-desk-raised)] px-2.5 py-0.5 rounded-full border border-[var(--color-desk-line)]">
              AI Gateway
            </span>
          </Link>

          <div className="flex items-center gap-x-4 gap-y-2 text-sm sm:gap-x-6">
            <div className="hidden items-center gap-x-6 md:flex font-bold text-xs uppercase tracking-wide">
              <Link
                className="text-[var(--color-muted-on-desk)] hover:text-[var(--color-paper-on-desk)] transition-colors"
                href="/connect"
              >
                How to use
              </Link>
              <Link
                className="text-[var(--color-muted-on-desk)] hover:text-[var(--color-paper-on-desk)] transition-colors"
                href="/pricing"
              >
                Pricing
              </Link>
              <Link
                className="text-[var(--color-muted-on-desk)] hover:text-[var(--color-paper-on-desk)] transition-colors"
                href="/faq"
              >
                FAQ
              </Link>
              <Link
                className="text-[var(--color-muted-on-desk)] hover:text-[var(--color-paper-on-desk)] transition-colors"
                href="/docs"
              >
                Docs
              </Link>
              <Link
                className="text-[var(--color-muted-on-desk)] hover:text-[var(--color-paper-on-desk)] transition-colors"
                href="/login"
              >
                Sign in
              </Link>
            </div>

            <Link
              href="/signup"
              className="rounded-lg bg-[var(--color-accent)] px-4 py-2 text-xs font-bold text-[var(--color-surface)] transition-all hover:bg-[var(--color-accent-dark)] active:scale-95 border border-[var(--color-accent-dark)] shadow-[2px_2px_0_var(--color-desk-line)]"
            >
              Request access →
            </Link>
          </div>
        </div>
      </header>

      {/* Main Content */}
      <main>{children}</main>

      {/* Footer — a raised desk panel, not another paper surface */}
      <footer className="mt-16 flex flex-wrap items-center justify-between gap-4 border border-[var(--color-desk-line)] bg-[var(--color-desk-raised)] rounded-2xl p-6 text-xs text-[var(--color-muted-on-desk)] font-bold">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <AegisLogo className="w-4 h-4 text-[var(--color-accent)]" />
            <span className="font-bold text-[var(--color-paper-on-desk)]">Aegis</span>
          </div>
          <span className="text-[var(--color-faint-on-desk)]">·</span>
          <span>The control plane for AI cost and governance</span>
        </div>
        <div className="flex items-center gap-5">
          <Link className="hover:text-[var(--color-paper-on-desk)] transition-colors" href="/connect">
            How to use
          </Link>
          <Link className="hover:text-[var(--color-paper-on-desk)] transition-colors" href="/pricing">
            Pricing
          </Link>
          <Link className="hover:text-[var(--color-paper-on-desk)] transition-colors" href="/faq">
            FAQ
          </Link>
          <Link className="hover:text-[var(--color-paper-on-desk)] transition-colors" href="/docs">
            API Docs
          </Link>
        </div>
      </footer>
    </div>
  );
}
