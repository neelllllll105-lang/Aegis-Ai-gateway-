"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { api, ApiError, type Organization } from "@/lib/api";

/**
 * Dashboard shell: sidebar navigation and an auth guard.
 *
 * The guard resolves the session once here rather than in each page. Every page would
 * otherwise repeat the check, and a page that forgot would render a broken empty state
 * instead of redirecting.
 */

const NAV = [
  { href: "/dashboard", label: "Overview" },
  { href: "/savings", label: "Savings" },
  { href: "/usage", label: "Usage" },
  { href: "/requests", label: "Requests" },
  { href: "/keys", label: "API keys" },
  { href: "/providers", label: "Providers" },
  { href: "/settings", label: "Settings" },
];

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const [org, setOrg] = useState<Organization | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    api
      .me()
      .then((response) => {
        if (cancelled) return;
        setOrg(response.organization);
        setState("ready");
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        if (caught instanceof ApiError && caught.isUnauthorized) {
          router.replace("/login");
          return;
        }
        // Anything else — the gateway being down, no database configured — is shown
        // rather than bounced to login, which would be a confusing lie.
        setMessage(
          caught instanceof ApiError
            ? caught.message
            : "Could not reach the Aegis API. Is the gateway running?",
        );
        setState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [router]);

  async function handleSignOut() {
    try {
      await api.logout();
    } finally {
      router.push("/login");
    }
  }

  if (state === "loading") {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <p className="text-sm text-[var(--color-ink-subtle)]">Loading…</p>
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="flex min-h-screen items-center justify-center px-6">
        <div className="card max-w-md p-6">
          <h1 className="text-base font-medium text-[var(--color-ink)]">
            Cannot reach the API
          </h1>
          <p className="mt-2 text-sm text-[var(--color-ink-muted)]">{message}</p>
          <p className="mt-4 text-xs text-[var(--color-ink-faint)]">
            Start the gateway with{" "}
            <code className="text-[var(--color-ink-subtle)]">
              cd apps/gateway &amp;&amp; cargo run
            </code>
            , then reload.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-screen">
      {/* Sidebar. Collapses to a horizontal bar on narrow screens rather than hiding
          behind a hamburger — there are only seven destinations. */}
      <aside className="hidden w-56 shrink-0 border-r border-[var(--color-line)] md:block">
        <div className="sticky top-0 flex h-screen flex-col p-4">
          <Link href="/dashboard" className="flex items-center gap-2 px-2 py-1">
            <svg
              width="18"
              height="18"
              viewBox="0 0 20 20"
              fill="none"
              aria-hidden="true"
              className="text-[var(--color-accent)]"
            >
              <path
                d="M10 1.5 3 4.2v5.3c0 4.2 2.9 7.5 7 9 4.1-1.5 7-4.8 7-9V4.2L10 1.5Z"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinejoin="round"
              />
              <path
                d="m6.8 9.8 2.3 2.3 4.1-4.6"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
            <span className="text-sm font-medium text-[var(--color-ink)]">Aegis</span>
          </Link>

          <nav className="mt-6 flex-1 space-y-0.5" aria-label="Dashboard">
            {NAV.map((item) => {
              const active = pathname === item.href;
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  aria-current={active ? "page" : undefined}
                  className={`block rounded-[var(--radius)] px-2 py-1.5 text-sm transition-colors ${
                    active
                      ? "bg-[var(--color-raised)] text-[var(--color-ink)]"
                      : "text-[var(--color-ink-subtle)] hover:bg-[var(--color-surface)] hover:text-[var(--color-ink-muted)]"
                  }`}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>

          <div className="border-t border-[var(--color-line)] pt-3">
            {org && (
              <div className="px-2 pb-2">
                <div className="truncate text-sm text-[var(--color-ink-muted)]">
                  {org.name}
                </div>
                <div className="text-xs capitalize text-[var(--color-ink-faint)]">
                  {org.plan} plan
                </div>
              </div>
            )}
            <button
              type="button"
              onClick={handleSignOut}
              className="w-full rounded-[var(--radius)] px-2 py-1.5 text-left text-sm text-[var(--color-ink-subtle)] transition-colors hover:bg-[var(--color-surface)] hover:text-[var(--color-ink-muted)]"
            >
              Sign out
            </button>
          </div>
        </div>
      </aside>

      <div className="min-w-0 flex-1">
        <nav
          className="flex gap-1 overflow-x-auto border-b border-[var(--color-line)] px-4 py-2 md:hidden"
          aria-label="Dashboard"
        >
          {NAV.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              aria-current={pathname === item.href ? "page" : undefined}
              className={`whitespace-nowrap rounded-[var(--radius)] px-2.5 py-1 text-sm ${
                pathname === item.href
                  ? "bg-[var(--color-raised)] text-[var(--color-ink)]"
                  : "text-[var(--color-ink-subtle)]"
              }`}
            >
              {item.label}
            </Link>
          ))}
        </nav>

        <main className="px-4 py-8 sm:px-8">
          <div className="mx-auto max-w-5xl">{children}</div>
        </main>
      </div>
    </div>
  );
}
