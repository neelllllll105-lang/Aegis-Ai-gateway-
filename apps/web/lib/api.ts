/**
 * Typed client for the Aegis management API.
 *
 * # Why every call goes through here
 *
 * Session authentication is an HTTP-only cookie, which means `credentials: "include"` on
 * every request and no token ever reaching JavaScript. Centralising that removes the
 * possibility of a component fetching directly, forgetting the flag, and appearing to be
 * logged out for reasons nobody can reproduce.
 *
 * It also means the gateway error envelope is unwrapped in exactly one place, so a
 * component receives a real message ("budget exceeded: $12.40 of $10.00") rather than
 * "Request failed with status 402".
 */

/** Base URL of the gateway. */
export function getApiUrl(): string {
  if (typeof window !== "undefined") {
    const hostname = window.location.hostname;
    const protocol = window.location.protocol;
    // In local dev / LAN mode, always target port 8080 on the EXACT same host the browser is viewing
    // (e.g. localhost -> localhost:8080, or 192.168.x.x -> 192.168.x.x:8080).
    // This ensures requests remain same-site so SameSite=Lax session cookies are preserved.
    if (
      hostname === "localhost" ||
      hostname === "127.0.0.1" ||
      hostname.startsWith("192.168.") ||
      hostname.startsWith("10.") ||
      hostname.startsWith("172.")
    ) {
      return `${protocol}//${hostname}:8080`;
    }
  }
  return process.env.NEXT_PUBLIC_AEGIS_API_URL ?? "http://localhost:8080";
}

export const API_URL =
  process.env.NEXT_PUBLIC_AEGIS_API_URL ?? "http://localhost:8080";

/** The gateway error envelope, per MASTER_BUILD.md Part 12. */
export interface ApiErrorBody {
  error: {
    type: string;
    message: string;
    docs_url: string;
    upgrade_url?: string;
  };
}

/** An error from the gateway, carrying its machine-readable type. */
export class ApiError extends Error {
  readonly status: number;
  /** Stable error type, e.g. `budget_exceeded`. Branch on this, not the message. */
  readonly type: string;
  readonly docsUrl?: string;
  readonly upgradeUrl?: string;

  constructor(
    status: number,
    type: string,
    message: string,
    docsUrl?: string,
    upgradeUrl?: string,
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.type = type;
    this.docsUrl = docsUrl;
    this.upgradeUrl = upgradeUrl;
  }

  /** True when the user needs to sign in again. */
  get isUnauthorized(): boolean {
    return this.status === 401;
  }
}

interface RequestOptions {
  method?: string;
  body?: unknown;
  /** Server components must opt out of caching for per-user data. */
  cache?: RequestCache;
}

/**
 * Perform an API request.
 *
 * Throws {@link ApiError} on any non-2xx response so callers can use ordinary
 * try/catch rather than checking a status on every call site.
 */
export async function apiRequest<T>(
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const baseUrl = getApiUrl();
  const response = await fetch(`${baseUrl}${path}`, {
    method: options.method ?? "GET",
    headers: { "Content-Type": "application/json" },
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    // The session cookie is HTTP-only, so it only travels when credentials are included.
    credentials: "include",
    cache: options.cache ?? "no-store",
  });

  if (!response.ok) {
    let type = "unknown_error";
    let message = `Request failed with status ${response.status}`;
    let docsUrl: string | undefined;
    let upgradeUrl: string | undefined;

    try {
      const body = (await response.json()) as Partial<ApiErrorBody>;
      if (body.error) {
        type = body.error.type ?? type;
        message = body.error.message ?? message;
        docsUrl = body.error.docs_url;
        upgradeUrl = body.error.upgrade_url;
      }
    } catch {
      // A non-JSON error body (a proxy 502, say) leaves the status-based default,
      // which is still more useful than throwing a parse error over the real failure.
    }

    throw new ApiError(response.status, type, message, docsUrl, upgradeUrl);
  }

  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

export interface Organization {
  id: string;
  name: string;
  slug: string;
  plan: string;
  savings_share_bp: number;
  billing_email: string | null;
  zero_retention: boolean;
  content_capture: boolean;
  region: string;
  created_at: string;
}

export interface User {
  id: string;
  email: string;
  name: string | null;
  is_admin: boolean;
  email_verified_at: string | null;
  created_at: string;
}

export interface ApiKey {
  id: string;
  org_id: string;
  team_id: string | null;
  /**
   * The person this key was issued to, if any. `null` is a shared project or service
   * key — a real answer, not missing data. Traffic on an assigned key is attributed to
   * that person in usage records.
   */
  assigned_to_user_id: string | null;
  name: string;
  key_prefix: string;
  rate_limit_per_minute: number;
  monthly_budget_mc: number | null;
  allowed_models: string[] | null;
  last_used_at: string | null;
  expires_at: string | null;
  revoked_at: string | null;
  created_at: string;
}

export interface CreatedKey {
  /** The full key. Shown once, never retrievable again. */
  key: string;
  metadata: ApiKey;
  warning: string;
}

export interface UsageSummary {
  requests: number;
  cache_hits: number;
  input_tokens: number;
  output_tokens: number;
  baseline_cost_mc: number;
  actual_cost_mc: number;
  gross_savings_mc: number;
  aegis_fee_mc: number;
}

export interface UsageSummaryResponse {
  period: { start: string; end: string };
  summary: UsageSummary;
  derived: {
    savings_percent: number;
    cache_hit_rate: number;
    customer_net_mc: number;
  };
}

export interface RequestLogRow {
  request_id: string;
  requested_model: string;
  served_model: string;
  provider: string;
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens?: number;
  tokens_saved_by_compression?: number;
  baseline_cost_mc: number;
  actual_cost_mc: number;
  input_cost_mc?: number;
  output_cost_mc?: number;
  gross_savings_mc: number;
  latency_ms: number;
  cache_hit: boolean;
  cache_type: string | null;
  routing_reason: string;
  complexity_score_milli?: number | null;
  status_code: number;
  created_at: string;
}

export interface ProviderCredential {
  id: string;
  provider: string;
  key_hint: string | null;
  base_url: string | null;
  label: string | null;
  is_default: boolean;
  last_tested_at: string | null;
  last_test_ok: boolean | null;
  created_at: string;
}

export interface OrgResponse {
  organization: Organization;
  usage: {
    month_to_date_spend_mc: number;
    month_to_date_savings_mc: number;
    month_to_date_requests: number;
  };
}

export interface BillingPlan {
  plan: string;
  savings_share_bp: number;
  savings_share_percent: number;
  subscription_mc: number;
  limits: {
    requests_per_minute: number;
    monthly_request_allowance: number | null;
    byok: boolean;
  };
}


export interface ModelPrice {
  model_id: string;
  provider: string;
  tier: "cheap" | "mid" | "premium" | "frontier";
  input_per_mtok_mc: number;
  output_per_mtok_mc: number;
  blended_per_mtok_mc: number;
  cheapest_in_tier_mc: number;
  potential_saving_pct: number;
  context_window: number;
  supports_vision: boolean;
  supports_tools: boolean;
  is_active: boolean;
  /** Where this price came from. A number nobody can trace is a number nobody should bill against. */
  source: string;
}

export interface Team {
  id: string;
  org_id: string;
  name: string;
  monthly_budget_mc: number | null;
  created_at: string;
}

export interface Member {
  user_id: string;
  email: string;
  name: string | null;
  role: string;
  joined_at: string;
}

/**
 * One routing-policy rule. Matches `engine::policy::Rule` on the gateway: `when` accepts
 * `complexity` / `model_requested` / `team` / `requires_tools` / `min_input_tokens`,
 * `then` accepts `model_tier` / `max_model_tier` / `pin_model` / `deny` / `passthrough`.
 * A field the backend doesn't recognise is silently dropped by serde, not rejected — so a
 * typo here produces a rule that parses fine and matches nothing, which is exactly the
 * failure mode `RoutingPolicy::from_json`'s own doc comment warns about.
 */
export interface PolicyRule {
  when: Record<string, unknown>;
  then: Record<string, unknown>;
}

export interface Policy {
  id: string;
  org_id: string;
  name: string;
  rules: PolicyRule[];
  is_active: boolean;
  created_at: string;
}

export interface Budget {
  id: string;
  org_id: string;
  team_id: string | null;
  api_key_id: string | null;
  period: string;
  limit_mc: number;
  hard_limit: boolean;
  created_at: string;
}

export interface AnomalyReport {
  org_id: string;
  /** The spend being judged. */
  observed_mc: number;
  /** Mean daily spend over the baseline window. */
  baseline_mean_mc: number;
  baseline_stddev_mc: number;
  /** How many standard deviations from the mean. Zero when undefined. */
  z_score: number;
  is_anomalous: boolean;
  /** Why the detector reached its conclusion, in words an operator can act on. */
  explanation: string;
}

export interface ChargebackLine {
  cost_center: string;
  requests: number;
  spend_mc: number;
  savings_mc: number;
  fee_mc: number;
  /** Share of total organisation spend, as a percentage. */
  share_percent: number;
}

export interface ChargebackReport {
  org_id: string;
  period_start: string;
  period_end: string;
  lines: ChargebackLine[];
  total_spend_mc: number;
  /** Spend that could not be attributed to any cost center. */
  unattributed_mc: number;
}

export interface CreditsResponse {
  balance_mc: number;
  referral_code: string;
  referral_url: string;
  credit_per_referral_mc: number;
  terms: string;
}

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

export const api = {
  signup: (email: string, password: string, name?: string) =>
    apiRequest<{ user: User; organization: Organization }>("/api/auth/signup", {
      method: "POST",
      body: { email, password, name },
    }),

  login: (email: string, password: string) =>
    apiRequest<{ user: User; organizations: Organization[] }>("/api/auth/login", {
      method: "POST",
      body: { email, password },
    }),

  acceptInvite: (token: string, password: string, name?: string) =>
    apiRequest<{ user: User; organizations: Organization[]; message: string }>(
      "/api/auth/accept-invite",
      {
        method: "POST",
        body: { token, password, name },
      },
    ),

  logout: () => apiRequest<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),

  me: () =>
    apiRequest<{
      user: User | null;
      organization: Organization | null;
      role: string | null;
      is_admin: boolean;
    }>("/api/auth/me"),

  org: () => apiRequest<OrgResponse>("/api/org"),

  listKeys: () => apiRequest<{ keys: ApiKey[] }>("/api/keys"),

  createKey: (input: {
    name: string;
    rate_limit_per_minute?: number;
    monthly_budget_mc?: number;
    allowed_models?: string[];
    /**
     * Issue the key to a named person so their spend is attributed to them. Only an
     * owner or admin may name someone other than themselves; the gateway rejects it
     * otherwise, and rejects an assignee who is not a member of the organisation.
     */
    assigned_to_user_id?: string;
  }) => apiRequest<CreatedKey>("/api/keys", { method: "POST", body: input }),

  revokeKey: (id: string) =>
    apiRequest<{ revoked: boolean }>(`/api/keys/${id}`, { method: "DELETE" }),

  deleteKey: (id: string) =>
    apiRequest<{ deleted: boolean }>(`/api/keys/${id}?permanent=true`, { method: "DELETE" }),

  usageSummary: (start?: string, end?: string) => {
    const params = new URLSearchParams();
    if (start) params.set("start", start);
    if (end) params.set("end", end);
    const query = params.toString();
    return apiRequest<UsageSummaryResponse>(
      `/api/usage/summary${query ? `?${query}` : ""}`,
    );
  },

  requests: (limit = 100) =>
    apiRequest<{ requests: RequestLogRow[] }>(`/api/requests?limit=${limit}`),

  listProviders: () =>
    apiRequest<{ providers: ProviderCredential[] }>("/api/providers"),

  createProvider: (input: {
    provider: string;
    api_key: string;
    base_url?: string;
    label?: string;
  }) =>
    apiRequest<ProviderCredential>("/api/providers", {
      method: "POST",
      body: input,
    }),

  deleteProvider: (id: string) =>
    apiRequest<{ deleted: boolean }>(`/api/providers/${id}`, { method: "DELETE" }),

  testProvider: (id: string) =>
    apiRequest<{ ok: boolean; provider: string; error: string | null }>(
      `/api/providers/${id}/test`,
      { method: "POST" },
    ),

  billingPlan: () => apiRequest<BillingPlan>("/api/billing/plan"),

  /** URL for the CSV export. A direct link, so the browser handles the download. */

  models: () =>
    apiRequest<{ models: ModelPrice[]; count: number; active_count: number }>(
      "/api/models",
    ),

  listTeams: () => apiRequest<{ teams: Team[] }>("/api/org/teams"),

  createTeam: (name: string, monthly_budget_mc?: number | null) =>
    apiRequest<{ team: Team }>("/api/org/teams", {
      method: "POST",
      body: { name, monthly_budget_mc: monthly_budget_mc ?? null },
    }),

  deleteTeam: (id: string) =>
    apiRequest<void>(`/api/org/teams/${id}`, { method: "DELETE" }),

  listMembers: () => apiRequest<{ members: Member[] }>("/api/org/members"),

  inviteMember: (email: string, role: string) =>
    apiRequest<{
      invited: string;
      role: string;
      invite_url?: string;
      email_sent?: boolean;
    }>("/api/org/members/invite", { method: "POST", body: { email, role } }),

  removeMember: (userId: string) =>
    apiRequest<void>(`/api/org/members/${userId}`, { method: "DELETE" }),

  updateMemberRole: (userId: string, role: string) =>
    apiRequest<{ updated: boolean; role: string }>(`/api/org/members/${userId}`, {
      method: "PATCH",
      body: { role },
    }),

  listPolicies: () => apiRequest<{ policies: Policy[] }>("/api/policies"),

  createPolicy: (name: string, rules: PolicyRule[]) =>
    apiRequest<{ policy: Policy }>("/api/policies", {
      method: "POST",
      body: { name, rules },
    }),

  deletePolicy: (id: string) =>
    apiRequest<void>(`/api/policies/${id}`, { method: "DELETE" }),

  listBudgets: () => apiRequest<{ budgets: Budget[] }>("/api/budgets"),

  createBudget: (input: {
    team_id?: string | null;
    api_key_id?: string | null;
    period: string;
    limit_mc: number;
    hard_limit: boolean;
  }) => apiRequest<{ budget: Budget }>("/api/budgets", { method: "POST", body: input }),

  deleteBudget: (id: string) =>
    apiRequest<void>(`/api/budgets/${id}`, { method: "DELETE" }),

  anomalies: () => apiRequest<AnomalyReport>("/api/usage/anomalies"),

  chargeback: (start?: string, end?: string) => {
    const query = new URLSearchParams();
    if (start) query.set("start", start);
    if (end) query.set("end", end);
    const suffix = query.toString() ? `?${query}` : "";
    return apiRequest<ChargebackReport>(`/api/usage/chargeback${suffix}`);
  },

  chargebackCsvUrl: () => `${getApiUrl()}/api/usage/chargeback.csv`,

  credits: () => apiRequest<CreditsResponse>("/api/billing/credits"),

  claimReferral: (code: string) =>
    apiRequest<{ credited_mc: number; message: string }>("/api/billing/referral", {
      method: "POST",
      body: { code },
    }),

  savingsCsvUrl: () => `${getApiUrl()}/api/savings/report.csv`,
};
