"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import {
  api,
  ApiError,
  type Organization,
  type PlanFeature,
  type User,
} from "@/lib/api";
import { AegisLogo } from "@/components/ui";
import { AuthProvider, canManageKeysForRole, canWriteForRole } from "@/lib/auth-context";
import { OnboardingTour } from "@/components/onboarding-tour";

interface NavItem {
  href: string;
  label: string;
  icon: string;
  /** Omitted for a page every plan includes (Free through Enterprise). Present names the
   *  `PlanFeature` that must be included, per `GET /api/billing/plan` — the mapping lives
   *  once on the gateway (`billing::features`), never duplicated here. */
  feature?: PlanFeature;
}

/**
 * Grouped so the sidebar reads as three jobs rather than one flat list of ten links:
 * what happened (analyse), what it may do (control), and who we are (account).
 */
const NAV: { section: string; items: NavItem[] }[] = [
  {
    section: "Analyse",
    items: [
      { href: "/dashboard", label: "Overview", icon: "M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6" },
      { href: "/savings", label: "Savings & ROI", icon: "M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z", feature: "savings" },
      { href: "/usage", label: "Usage & Tokens", icon: "M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" },
      { href: "/requests", label: "Request Logs", icon: "M4 6h16M4 10h16M4 14h16M4 18h16" },
    ],
  },
  {
    section: "Control",
    items: [
      { href: "/keys", label: "API Keys", icon: "M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" },
      { href: "/providers", label: "Providers", icon: "M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10", feature: "byok" },
      { href: "/models", label: "Models", icon: "M4 7v10c0 2 1 3 3 3h10c2 0 3-1 3-3V7c0-2-1-3-3-3H7C5 4 4 5 4 7zM4 10h16M10 4v16" },
      { href: "/policies", label: "Policies", icon: "M9 12l2 2 4-4M7.835 4.697a3.42 3.42 0 001.946-.806 3.42 3.42 0 014.438 0 3.42 3.42 0 001.946.806 3.42 3.42 0 013.138 3.138 3.42 3.42 0 00.806 1.946 3.42 3.42 0 010 4.438 3.42 3.42 0 00-.806 1.946 3.42 3.42 0 01-3.138 3.138 3.42 3.42 0 00-1.946.806 3.42 3.42 0 01-4.438 0 3.42 3.42 0 00-1.946-.806 3.42 3.42 0 01-3.138-3.138 3.42 3.42 0 00-.806-1.946 3.42 3.42 0 010-4.438 3.42 3.42 0 00.806-1.946 3.42 3.42 0 013.138-3.138z", feature: "policies" },
      { href: "/budgets", label: "Budgets", icon: "M3 6a2 2 0 012-2h14a2 2 0 012 2v2H3V6zm0 4h18v8a2 2 0 01-2 2H5a2 2 0 01-2-2v-8zm12 4h3", feature: "budgets" },
    ],
  },
  {
    section: "Account",
    items: [
      { href: "/team", label: "People & Teams", icon: "M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z", feature: "team_management" },
      { href: "/billing", label: "Billing", icon: "M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z" },
      { href: "/settings", label: "Settings", icon: "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" },
    ],
  },
];

export default function DashboardShell({
  children,
}: {
  children: React.ReactNode;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const [org, setOrg] = useState<Organization | null>(null);
  const [user, setUser] = useState<User | null>(null);
  const [role, setRole] = useState<string | null>(null);
  const [isAdmin, setIsAdmin] = useState(false);
  const [planFeatures, setPlanFeatures] = useState<Partial<Record<PlanFeature, boolean>>>({});
  const [state, setState] = useState<"loading" | "ready" | "error">("loading");
  const [message, setMessage] = useState<string | null>(null);
  const [showTour, setShowTour] = useState(false);

  useEffect(() => {
    let cancelled = false;

    // Both requests need a session, and the nav must not flash gated items before
    // billingPlan() resolves — so both are awaited before the shell ever renders, same as
    // me() alone did before this plan-gating existed.
    Promise.all([api.me(), api.billingPlan().catch(() => null)])
      .then(([response, plan]) => {
        if (cancelled) return;
        setOrg(response.organization);
        setUser(response.user);
        setRole(response.role);
        setIsAdmin(response.is_admin);
        setPlanFeatures(plan?.features ?? {});
        setState("ready");

        const isTeamOrEnterprise = plan?.plan === "team" || plan?.plan === "enterprise";
        const isOwnerOrAdmin = response.role === "owner" || response.role === "admin";
        if (isTeamOrEnterprise && isOwnerOrAdmin && !response.user?.onboarding_completed_at) {
          setShowTour(true);
        }
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        if (caught instanceof ApiError && caught.isUnauthorized) {
          router.replace("/login");
          return;
        }
        setMessage(
          caught instanceof ApiError
            ? caught.message
            : "Could not reach the Aegis API. Is the gateway running on port 8080?",
        );
        setState("error");
      });

    return () => {
      cancelled = true;
    };
  }, [router]);

  const visibleNav = NAV.map((group) => ({
    ...group,
    items: group.items.filter((item) => !item.feature || planFeatures[item.feature]),
  })).filter((group) => group.items.length > 0);
  const visibleFlatNav = visibleNav.flatMap((group) => group.items);

  async function handleTourFinished() {
    setShowTour(false);
    try {
      await api.completeOnboarding();
    } catch {
      // Best-effort: the tour still closes for this session even if the write fails: a
      // failed "mark seen" call must not trap someone behind a walkthrough they already
      // finished.
    }
  }

  async function handleSignOut() {
    try {
      await api.logout();
    } finally {
      router.push("/login");
    }
  }

  if (state === "loading") {
    return (
      <div className="flex min-h-screen items-center justify-center bg-[var(--color-bg)]">
        <div className="flex items-center gap-2 text-xs font-bold text-[var(--color-paper-on-desk)]">
          <span className="flex h-2 w-2 rounded-full bg-[var(--color-accent)] animate-pulse" />
          Loading dashboard session…
        </div>
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="flex min-h-screen items-center justify-center px-6 bg-[var(--color-bg)]">
        <div className="rounded-2xl border-[1.5px] border-[var(--color-ink)] bg-[var(--color-surface)] max-w-md p-6 shadow-[4px_4px_0_var(--shadow-color)]">
          <div className="flex items-center gap-2 text-[var(--color-amber)] mb-2">
            <svg className="w-5 h-5" viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z" clipRule="evenodd" />
            </svg>
            <h1 className="font-serif text-base font-semibold text-[var(--color-ink)]">
              API Connection Notice
            </h1>
          </div>
          <p className="text-xs text-[var(--color-muted)] leading-relaxed font-medium">{message}</p>
          <div className="mt-4 rounded-xl bg-[var(--color-surface2)] p-3 border border-[var(--color-ink)]">
            <p className="text-[11px] text-[var(--color-ink)] font-mono font-bold">
              cd apps/gateway &amp;&amp; cargo run --bin aegis-gateway
            </p>
          </div>
          <div className="mt-4 flex justify-end">
            <button
              onClick={() => window.location.reload()}
              className="rounded-xl bg-[var(--color-accent)] px-4 py-2 text-xs font-bold text-[var(--color-surface)] shadow-[2px_2px_0_var(--shadow-color)] hover:bg-[var(--color-accent-dark)]"
            >
              Retry Connection
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <AuthProvider
      value={{
        user,
        organization: org,
        role,
        isAdmin,
        canWrite: canWriteForRole(role),
        canManageKeys: canManageKeysForRole(role),
        planFeatures,
      }}
    >
    <div className="flex min-h-screen bg-[var(--color-bg)] text-[var(--color-paper-on-desk)]">
      {/* Sidebar — a raised desk panel; the active page is a paper tab set into it */}
      <aside className="hidden w-60 shrink-0 border-r border-[var(--color-desk-line)] bg-[var(--color-desk-raised)] md:block">
        <div className="sticky top-0 flex h-screen flex-col p-4">
          <Link href="/dashboard" className="flex items-center gap-2.5 px-2 py-2 group">
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-[var(--color-surface2)] border-[1.5px] border-[var(--color-ink)] text-[var(--color-ink)]">
              <AegisLogo className="w-5 h-5 text-[var(--color-accent)]" />
            </div>
            <span className="font-serif text-sm font-semibold tracking-tight text-[var(--color-paper-on-desk)]">
              Aegis Dashboard
            </span>
          </Link>

          <nav className="mt-6 flex-1 space-y-5 overflow-y-auto" aria-label="Dashboard">
            {visibleNav.map((group) => (
              <div key={group.section}>
                <div className="px-3 pb-1.5 font-mono text-[10px] font-bold uppercase tracking-[0.18em] text-[var(--color-faint-on-desk)]">
                  {group.section}
                </div>
                <div className="space-y-0.5">
                  {group.items.map((item) => {
                    const active = pathname === item.href;
                    return (
                      <Link
                        key={item.href}
                        href={item.href}
                        aria-current={active ? "page" : undefined}
                        className={`flex items-center gap-2.5 rounded-xl px-3 py-2 text-xs font-bold transition-all ${
                          active
                            ? "bg-[var(--color-surface)] text-[var(--color-ink)] border-[1.5px] border-[var(--color-ink)] shadow-[2px_2px_0_var(--color-desk-line)]"
                            : "text-[var(--color-muted-on-desk)] hover:bg-[var(--color-bg)] hover:text-[var(--color-paper-on-desk)]"
                        }`}
                      >
                        <svg className="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth="2" d={item.icon} />
                        </svg>
                        <span>{item.label}</span>
                      </Link>
                    );
                  })}
                </div>
              </div>
            ))}
          </nav>

          <div className="border-t border-[var(--color-desk-line)] pt-3">
            {org && (
              <div className="rounded-xl bg-[var(--color-surface)] p-2.5 mb-2 border-[1.5px] border-[var(--color-ink)]">
                <div className="truncate text-xs font-bold text-[var(--color-ink)]">
                  {org.name}
                </div>
                <div className="text-[10px] capitalize text-[var(--color-accent)] font-bold">
                  {org.plan} tier
                </div>
                {/* Whoever ends up on this dashboard should never have to wonder whose
                    account they're looking at — the identity and role are always visible,
                    right next to the one control that leaves it. */}
                {user && (
                  <div className="mt-2 border-t border-[var(--color-desk-line)] pt-2">
                    <div className="truncate text-[10px] text-[var(--color-muted-light)]">
                      Signed in as
                    </div>
                    <div className="truncate text-[11px] font-bold text-[var(--color-ink)]">
                      {user.name ?? user.email}
                    </div>
                    {role && (
                      <div className="text-[10px] capitalize text-[var(--color-muted-light)]">
                        {role}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}
            <button
              type="button"
              onClick={handleSignOut}
              className="w-full rounded-xl px-2.5 py-1.5 text-left text-xs font-bold text-[var(--color-muted-on-desk)] transition-colors hover:bg-[var(--color-bg)] hover:text-[var(--color-paper-on-desk)]"
            >
              Sign out
            </button>
          </div>
        </div>
      </aside>

      {/* Main Content Area */}
      <div className="min-w-0 flex-1">
        {/* Mobile Header Nav */}
        <nav
          className="flex gap-1 overflow-x-auto border-b border-[var(--color-desk-line)] bg-[var(--color-desk-raised)] px-4 py-2.5 md:hidden"
          aria-label="Dashboard"
        >
          {visibleFlatNav.map((item) => (
            <Link
              key={item.href}
              href={item.href}
              aria-current={pathname === item.href ? "page" : undefined}
              className={`whitespace-nowrap rounded-xl px-3 py-1.5 text-xs font-bold ${
                pathname === item.href
                  ? "bg-[var(--color-surface)] text-[var(--color-ink)] border-[1.5px] border-[var(--color-ink)]"
                  : "text-[var(--color-muted-on-desk)] hover:bg-[var(--color-bg)] hover:text-[var(--color-paper-on-desk)]"
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
    {showTour && <OnboardingTour onFinish={handleTourFinished} />}
    </AuthProvider>
  );
}
