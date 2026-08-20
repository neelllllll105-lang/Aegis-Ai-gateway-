-- Aegis initial schema.
--
-- Implements MASTER_BUILD.md Part 4. Migrations are the source of truth for the schema.
--
-- Two conventions run through everything below:
--
--   1. Money is BIGINT micro-cents (1 cent = 10_000). No NUMERIC, no floats, anywhere a
--      value can reach an invoice. Principle 9.
--   2. Every tenant-owned row carries org_id, and every application query filters on it.
--      That is the isolation boundary of Part 13 item 2.

-- gen_random_uuid() lives here on PostgreSQL 12; native from 13, but requesting the
-- extension is harmless and keeps older instances working.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- ============================================================================
-- USERS & AUTH
-- ============================================================================

CREATE TABLE users (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email             VARCHAR(255) UNIQUE NOT NULL,
    email_verified_at TIMESTAMPTZ,
    -- argon2id PHC string. NULL for OAuth-only accounts, which must never fall back to
    -- password auth.
    password_hash     VARCHAR(255),
    name              VARCHAR(255),
    avatar_url        TEXT,
    is_admin          BOOLEAN NOT NULL DEFAULT false,
    -- TOTP secret for admin 2FA (Phase 6), encrypted at rest.
    totp_secret_encrypted BYTEA,
    totp_enabled      BOOLEAN NOT NULL DEFAULT false,
    disabled_at       TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at        TIMESTAMPTZ
);
CREATE INDEX idx_users_email ON users (LOWER(email)) WHERE deleted_at IS NULL;

CREATE TABLE sessions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- SHA-256 of the session token. The token itself is never stored.
    token_hash  VARCHAR(64) NOT NULL UNIQUE,
    ip_address  INET,
    user_agent  TEXT,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_sessions_token ON sessions (token_hash);
CREATE INDEX idx_sessions_expiry ON sessions (expires_at);

-- Single-use tokens for email verification and password reset.
CREATE TABLE auth_tokens (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash  VARCHAR(64) NOT NULL UNIQUE,
    purpose     VARCHAR(32) NOT NULL CHECK (purpose IN ('email_verify', 'password_reset')),
    expires_at  TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_auth_tokens_hash ON auth_tokens (token_hash);

-- ============================================================================
-- ORGANIZATIONS (TENANCY ROOT)
-- ============================================================================

CREATE TABLE organizations (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name               VARCHAR(255) NOT NULL,
    slug               VARCHAR(255) UNIQUE NOT NULL,
    plan               VARCHAR(50) NOT NULL DEFAULT 'free'
                       CHECK (plan IN ('free', 'pro', 'team', 'enterprise', 'api')),
    -- Basis points rather than a fraction: the fee calculation is integer-only.
    -- 2000 = 20% (Pro), 1500 = 15% (Team), 1000 = 10% (Enterprise), 0 = Free.
    savings_share_bp   INTEGER NOT NULL DEFAULT 2000
                       CHECK (savings_share_bp BETWEEN 0 AND 10000),
    billing_email      VARCHAR(255),
    -- Principle 4. zero_retention disables caching AND content capture outright.
    zero_retention     BOOLEAN NOT NULL DEFAULT false,
    content_capture    BOOLEAN NOT NULL DEFAULT false,
    -- Data residency pin (Phase 6).
    region             VARCHAR(32) NOT NULL DEFAULT 'eu-central',
    stripe_customer_id VARCHAR(255),
    settings           JSONB NOT NULL DEFAULT '{}',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Content capture is only meaningful when retention is permitted at all; this makes
    -- the contradictory combination unrepresentable rather than merely discouraged.
    CONSTRAINT retention_settings_are_consistent
        CHECK (NOT (zero_retention AND content_capture))
);
CREATE INDEX idx_orgs_slug ON organizations (slug);
CREATE INDEX idx_orgs_plan ON organizations (plan);

CREATE TABLE org_memberships (
    org_id     UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role       VARCHAR(50) NOT NULL DEFAULT 'member'
               CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    invited_by UUID REFERENCES users (id),
    joined_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, user_id)
);
CREATE INDEX idx_memberships_user ON org_memberships (user_id);

CREATE TABLE teams (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id               UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    name                 VARCHAR(255) NOT NULL,
    monthly_budget_mc    BIGINT CHECK (monthly_budget_mc IS NULL OR monthly_budget_mc >= 0),
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (org_id, name)
);
CREATE INDEX idx_teams_org ON teams (org_id);

CREATE TABLE team_memberships (
    team_id UUID NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (team_id, user_id)
);

-- ============================================================================
-- API KEYS
-- ============================================================================

CREATE TABLE api_keys (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id                UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    team_id               UUID REFERENCES teams (id) ON DELETE SET NULL,
    created_by            UUID REFERENCES users (id),
    name                  VARCHAR(255) NOT NULL,
    -- First 16 characters, for display. Enough to tell keys apart, useless as a credential.
    key_prefix            VARCHAR(20) NOT NULL,
    -- SHA-256 of the full key. The only stored form. Part 9 item 1.
    key_hash              VARCHAR(64) NOT NULL UNIQUE,
    scopes                JSONB NOT NULL DEFAULT '["chat"]',
    rate_limit_per_minute INTEGER NOT NULL DEFAULT 60 CHECK (rate_limit_per_minute > 0),
    monthly_budget_mc     BIGINT CHECK (monthly_budget_mc IS NULL OR monthly_budget_mc >= 0),
    -- NULL means every model the plan allows.
    allowed_models        JSONB,
    last_used_at          TIMESTAMPTZ,
    expires_at            TIMESTAMPTZ,
    revoked_at            TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_api_keys_hash ON api_keys (key_hash);
CREATE INDEX idx_api_keys_prefix ON api_keys (key_prefix);
CREATE INDEX idx_api_keys_org ON api_keys (org_id) WHERE revoked_at IS NULL;

-- ============================================================================
-- PROVIDER CREDENTIALS (BYOK)
-- ============================================================================

CREATE TABLE provider_credentials (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id        UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    provider      VARCHAR(50) NOT NULL,
    -- AES-256-GCM(master_key, provider_key) as nonce || ciphertext || tag.
    -- The master key never appears in this database. Part 9 item 2.
    encrypted_key BYTEA NOT NULL,
    -- Last 4 characters, so a customer can identify which key this is.
    key_hint      VARCHAR(10),
    base_url      TEXT,
    label         VARCHAR(255),
    is_default    BOOLEAN NOT NULL DEFAULT false,
    last_tested_at TIMESTAMPTZ,
    last_test_ok  BOOLEAN,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_credentials_org ON provider_credentials (org_id, provider);
-- At most one default per provider per organisation.
CREATE UNIQUE INDEX idx_credentials_default
    ON provider_credentials (org_id, provider) WHERE is_default;

-- ============================================================================
-- ROUTING POLICIES
-- ============================================================================

CREATE TABLE routing_policies (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    name       VARCHAR(255) NOT NULL,
    -- Ordered [{"when": {...}, "then": {...}}] rules; first match wins.
    rules      JSONB NOT NULL,
    is_active  BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_policies_org ON routing_policies (org_id) WHERE is_active;

-- ============================================================================
-- USAGE RECORDS — THE BILLING SOURCE OF TRUTH
-- ============================================================================
--
-- Partitioned monthly by created_at. Partitions are created ahead of time by
-- workers::usage_writer; a missing partition would reject inserts, so the worker keeps
-- the next two months live at all times.

CREATE TABLE usage_records (
    id                  BIGSERIAL,
    -- Idempotency key. The unique index below is what makes a redelivered stream entry
    -- harmless rather than a double charge.
    request_id          UUID NOT NULL,
    org_id              UUID NOT NULL,
    api_key_id          UUID,
    team_id             UUID,

    requested_model     VARCHAR(100) NOT NULL,
    served_model        VARCHAR(100) NOT NULL,
    provider            VARCHAR(50) NOT NULL,

    input_tokens        INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens       INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    tokens_estimated    BOOLEAN NOT NULL DEFAULT false,

    -- Micro-cents. baseline is what the requested model would have cost; actual is what
    -- we paid (zero on a cache hit).
    baseline_cost_mc    BIGINT NOT NULL DEFAULT 0,
    actual_cost_mc      BIGINT NOT NULL DEFAULT 0,
    gross_savings_mc    BIGINT NOT NULL DEFAULT 0 CHECK (gross_savings_mc >= 0),
    aegis_fee_mc        BIGINT NOT NULL DEFAULT 0 CHECK (aegis_fee_mc >= 0),

    latency_ms          INTEGER NOT NULL DEFAULT 0,
    -- Our own overhead, in MICROSECONDS. Part 13 item 4: displayed, never gamed.
    -- Integer rather than NUMERIC because sub-millisecond resolution is the whole point
    -- and a scaled integer needs no decimal type, no extra dependency, and no rounding
    -- discussion. Divide by 1000 for milliseconds at the presentation layer.
    gateway_overhead_us INTEGER NOT NULL DEFAULT 0 CHECK (gateway_overhead_us >= 0),

    cache_hit           BOOLEAN NOT NULL DEFAULT false,
    cache_type          VARCHAR(10) CHECK (cache_type IN ('exact', 'semantic')),

    routing_reason      VARCHAR(32) NOT NULL DEFAULT 'passthrough',
    -- Classifier score in THOUSANDTHS (0..1000), for the same reason.
    complexity_score_milli SMALLINT CHECK (complexity_score_milli IS NULL
                                           OR complexity_score_milli BETWEEN 0 AND 1000),
    tokens_saved_by_compression INTEGER NOT NULL DEFAULT 0,

    status_code         INTEGER NOT NULL,
    error_type          VARCHAR(100),

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, created_at),
    -- The fee can never exceed the saving it is charged on.
    CONSTRAINT fee_within_savings CHECK (aegis_fee_mc <= gross_savings_mc),
    -- A cache hit costs nothing.
    CONSTRAINT cache_hits_are_free CHECK (NOT cache_hit OR actual_cost_mc = 0),
    CONSTRAINT cache_type_matches_hit CHECK (
        (cache_hit AND cache_type IS NOT NULL) OR (NOT cache_hit AND cache_type IS NULL)
    )
) PARTITION BY RANGE (created_at);

CREATE INDEX idx_usage_org_time ON usage_records (org_id, created_at DESC);
CREATE INDEX idx_usage_key_time ON usage_records (api_key_id, created_at DESC);
CREATE INDEX idx_usage_team_time ON usage_records (team_id, created_at DESC);
CREATE UNIQUE INDEX idx_usage_request_id ON usage_records (request_id, created_at);

-- Daily rollups. Dashboards read these rather than scanning raw records; raw rows are
-- retained 24 months, rollups indefinitely.
CREATE TABLE daily_aggregates (
    org_id           UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    day              DATE NOT NULL,
    served_model     VARCHAR(100) NOT NULL,
    -- A sentinel rather than NULL: PostgreSQL cannot put an expression in a primary key,
    -- and NULL would make the org-wide row unmatchable by an equality upsert.
    team_id          UUID NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
    requests         BIGINT NOT NULL DEFAULT 0,
    cache_hits       BIGINT NOT NULL DEFAULT 0,
    input_tokens     BIGINT NOT NULL DEFAULT 0,
    output_tokens    BIGINT NOT NULL DEFAULT 0,
    baseline_cost_mc BIGINT NOT NULL DEFAULT 0,
    actual_cost_mc   BIGINT NOT NULL DEFAULT 0,
    gross_savings_mc BIGINT NOT NULL DEFAULT 0,
    aegis_fee_mc     BIGINT NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, day, served_model, team_id)
);
CREATE INDEX idx_daily_org_day ON daily_aggregates (org_id, day DESC);

-- ============================================================================
-- BUDGETS & ALERTS
-- ============================================================================

CREATE TABLE budgets (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    team_id      UUID REFERENCES teams (id) ON DELETE CASCADE,
    api_key_id   UUID REFERENCES api_keys (id) ON DELETE CASCADE,
    period       VARCHAR(20) NOT NULL DEFAULT 'monthly' CHECK (period IN ('daily', 'monthly')),
    limit_mc     BIGINT NOT NULL CHECK (limit_mc >= 0),
    -- true rejects requests past the limit; false only alerts.
    hard_limit   BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_budgets_org ON budgets (org_id);

CREATE TABLE budget_alerts (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    budget_id         UUID NOT NULL REFERENCES budgets (id) ON DELETE CASCADE,
    threshold_pct     INTEGER NOT NULL CHECK (threshold_pct BETWEEN 1 AND 200),
    channel           VARCHAR(50) NOT NULL DEFAULT 'email'
                      CHECK (channel IN ('email', 'slack', 'webhook')),
    destination       TEXT,
    -- Used to suppress repeat alerts within the same period.
    last_triggered_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- AUDIT LOG (APPEND-ONLY)
-- ============================================================================
--
-- No UPDATE or DELETE grant is issued to the application role in production; see
-- docs/runbooks/deploy.md. Retention is 7 years.

CREATE TABLE audit_logs (
    id            BIGSERIAL PRIMARY KEY,
    org_id        UUID NOT NULL,
    user_id       UUID,
    action        VARCHAR(100) NOT NULL,
    resource_type VARCHAR(50) NOT NULL,
    resource_id   UUID,
    metadata      JSONB,
    ip_address    INET,
    user_agent    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_audit_org_time ON audit_logs (org_id, created_at DESC);
CREATE INDEX idx_audit_action ON audit_logs (action, created_at DESC);

-- ============================================================================
-- BILLING
-- ============================================================================

CREATE TABLE invoices (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id              UUID NOT NULL REFERENCES organizations (id),
    period_start        DATE NOT NULL,
    period_end          DATE NOT NULL,
    status              VARCHAR(50) NOT NULL DEFAULT 'draft'
                        CHECK (status IN ('draft', 'pending', 'paid', 'failed', 'refunded', 'void')),
    subscription_mc     BIGINT NOT NULL DEFAULT 0,
    savings_fee_mc      BIGINT NOT NULL DEFAULT 0,
    tax_mc              BIGINT NOT NULL DEFAULT 0,
    total_mc            BIGINT NOT NULL DEFAULT 0,
    -- Reproduced on the invoice so a customer can audit the fee without our dashboard.
    gross_savings_mc    BIGINT NOT NULL DEFAULT 0,
    requests            BIGINT NOT NULL DEFAULT 0,
    pdf_url             TEXT,
    stripe_invoice_id   VARCHAR(255),
    finalized_at        TIMESTAMPTZ,
    paid_at             TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (org_id, period_start, period_end)
);
CREATE INDEX idx_invoices_org ON invoices (org_id, period_start DESC);

CREATE TABLE referral_credits (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    referred_org_id UUID REFERENCES organizations (id) ON DELETE SET NULL,
    amount_mc      BIGINT NOT NULL CHECK (amount_mc > 0),
    reason         VARCHAR(100) NOT NULL,
    consumed_at    TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_credits_org ON referral_credits (org_id) WHERE consumed_at IS NULL;

-- ============================================================================
-- MODEL PRICING (VERSIONED)
-- ============================================================================
--
-- Versioned by effective_from/effective_to so an invoice can always be recomputed with
-- the prices that were in force when the request ran. Part 13 item 8.

CREATE TABLE model_pricing (
    id                         BIGSERIAL PRIMARY KEY,
    model_id                   VARCHAR(100) NOT NULL,
    provider                   VARCHAR(50) NOT NULL,
    display_name               VARCHAR(255) NOT NULL,
    tier                       VARCHAR(20) NOT NULL
                               CHECK (tier IN ('cheap', 'mid', 'premium', 'frontier')),
    -- Micro-cents per million tokens.
    input_cost_per_mtok_mc     BIGINT NOT NULL CHECK (input_cost_per_mtok_mc >= 0),
    output_cost_per_mtok_mc    BIGINT NOT NULL CHECK (output_cost_per_mtok_mc >= 0),
    context_window             INTEGER NOT NULL CHECK (context_window > 0),
    supports_tools             BOOLEAN NOT NULL DEFAULT false,
    supports_vision            BOOLEAN NOT NULL DEFAULT false,
    is_active                  BOOLEAN NOT NULL DEFAULT true,
    -- Dated provenance for every number. Required, not optional.
    source                     TEXT NOT NULL,
    effective_from             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_to               TIMESTAMPTZ
);
CREATE INDEX idx_pricing_lookup ON model_pricing (model_id, effective_from DESC);
-- Exactly one current price per model.
CREATE UNIQUE INDEX idx_pricing_current
    ON model_pricing (model_id) WHERE effective_to IS NULL;

CREATE TABLE model_aliases (
    alias      VARCHAR(100) PRIMARY KEY,
    model_id   VARCHAR(100) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- PROVIDER HEALTH
-- ============================================================================

CREATE TABLE provider_health_events (
    id          BIGSERIAL PRIMARY KEY,
    provider    VARCHAR(50) NOT NULL,
    event_type  VARCHAR(50) NOT NULL CHECK (event_type IN ('outage', 'degraded', 'recovered')),
    details     JSONB,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ
);
CREATE INDEX idx_health_provider ON provider_health_events (provider, started_at DESC);

-- ============================================================================
-- OPTIONAL CONTENT CAPTURE (OFF BY DEFAULT)
-- ============================================================================
--
-- Written ONLY when organizations.content_capture is true AND zero_retention is false —
-- a combination the CHECK constraint on organizations already makes impossible to set
-- incorrectly. Encrypted with a per-tenant key derived via HKDF from the master key.

CREATE TABLE captured_content (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id             UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    request_id         UUID NOT NULL,
    prompt_encrypted   BYTEA,
    response_encrypted BYTEA,
    captured_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_captured_org ON captured_content (org_id, captured_at DESC);

-- ============================================================================
-- ENTERPRISE: SSO & SCIM (Phase 6)
-- ============================================================================

CREATE TABLE sso_connections (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    protocol       VARCHAR(20) NOT NULL CHECK (protocol IN ('oidc', 'saml')),
    issuer         TEXT NOT NULL,
    client_id      TEXT,
    client_secret_encrypted BYTEA,
    metadata_xml   TEXT,
    certificate    TEXT,
    email_domain   VARCHAR(255),
    is_active      BOOLEAN NOT NULL DEFAULT true,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (org_id, protocol)
);

CREATE TABLE scim_tokens (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     UUID NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);
