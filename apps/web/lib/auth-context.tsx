"use client";

import { createContext, useContext, type ReactNode } from "react";
import type { Organization, PlanFeature, User } from "@/lib/api";

/**
 * The signed-in caller's identity and standing — fetched once by `DashboardShell` (which
 * already calls `GET /api/auth/me` to decide whether to render the dashboard at all) and
 * handed down from there, so every page can gate its own controls without a second fetch.
 *
 * The three derived flags mirror the gateway's own guards exactly — `middleware/auth.rs`'s
 * `AuthContext::can_write()` and `routes/management.rs`'s `require_writer`/
 * `require_key_writer` — on purpose. A page hiding a button the backend would still refuse
 * is UX; a page hiding a button the backend would *allow* is a bug with a different shape.
 * Keeping the two definitions side by side (see the doc comments below) is what keeps them
 * from drifting apart.
 */
export interface AuthState {
  user: User | null;
  organization: Organization | null;
  role: string | null;
  isAdmin: boolean;
  /**
   * Mirrors `require_writer` on the gateway: organisation settings, members, teams,
   * policies, providers, budgets. `owner` or `admin` only — never true for a plain member
   * or viewer, and never true from an API key (this page is always reached via a session).
   */
  canWrite: boolean;
  /**
   * Mirrors `require_key_writer`: creating, renaming, or revoking API keys. Slightly wider
   * than `canWrite` — `owner`, `admin`, or `member` — because a member managing their own
   * keys is expected; only assigning a key to *someone else* additionally needs `canWrite`,
   * enforced server-side regardless of what this page shows.
   */
  canManageKeys: boolean;
  /**
   * Every dashboard feature this organisation's plan includes, exactly as
   * `GET /api/billing/plan` reported it — never computed from `organization.plan` here, so
   * the mapping lives in one place (`billing::features` on the gateway) and the frontend
   * cannot drift from what the backend actually enforces. Hiding a nav item or page this
   * doesn't include is UX; the write itself is still refused server-side either way.
   */
  planFeatures: Partial<Record<PlanFeature, boolean>>;
}

const AuthContext = createContext<AuthState | null>(null);

export function AuthProvider({
  value,
  children,
}: {
  value: AuthState;
  children: ReactNode;
}) {
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/**
 * The current dashboard session's identity and permissions.
 *
 * Only usable under `DashboardShell`, which is every page in the `(dashboard)` route
 * group — throwing on a missing provider catches a page rendered outside that layout
 * during development rather than silently defaulting to "no permissions" or "all
 * permissions," both of which would hide a real wiring mistake.
 */
export function useAuth(): AuthState {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth() called outside DashboardShell's AuthProvider");
  }
  return context;
}

/** `owner` or `admin` only. */
export function canWriteForRole(role: string | null): boolean {
  return role === "owner" || role === "admin";
}

/** `owner`, `admin`, or `member` — everyone except `viewer`. */
export function canManageKeysForRole(role: string | null): boolean {
  return role === "owner" || role === "admin" || role === "member";
}

/**
 * Whether the current session's plan includes `feature`. Convenience wrapper over
 * `useAuth().planFeatures` for the common case of gating one page or nav item.
 */
export function useHasFeature(feature: PlanFeature): boolean {
  return useAuth().planFeatures[feature] === true;
}
