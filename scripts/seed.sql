-- Development seed data.
--
--   psql "$DATABASE_URL" -f scripts/seed.sql
--
-- Creates one organisation, one owner, one API key, and the model pricing table.
--
-- # About the prices below
--
-- Every row carries a `source` naming the provider and the date it was checked, because
-- Part 13 item 8 requires every cost number to be traceable. **These figures were
-- transcribed for development and have NOT been verified against live provider price
-- sheets.** Before billing a real customer, follow `docs/runbooks/pricing-update.md` and
-- confirm each row. A wrong number here produces a wrong invoice, which Part 13 item 1
-- calls trust-destroying.
--
-- Costs are micro-cents per million tokens: $2.50/Mtok = 250 cents = 2_500_000 µ¢.

BEGIN;

-- ============================================================================
-- DEVELOPMENT ORGANISATION
-- ============================================================================

-- Idempotent: re-running the seed must not create a second copy of everything.
INSERT INTO users (id, email, name, password_hash, email_verified_at, is_admin)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'dev@aegis.local',
    'Development User',
    -- argon2id hash of "aegis-development-password". Development only; this account
    -- cannot exist in production because the seed is never run there.
    '$argon2id$v=19$m=19456,t=2,p=1$c2VlZHNlZWRzZWVkc2VlZA$Zx8Qb4hMhLxJmYq0dJZ7lGkVn3wCmQZ4rXqJ8xNvB1E',
    NOW(),
    true
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO organizations (id, name, slug, plan, savings_share_bp, billing_email)
VALUES (
    '00000000-0000-0000-0000-000000000010',
    'Development Org',
    'development',
    'pro',
    2000,
    'dev@aegis.local'
)
ON CONFLICT (id) DO NOTHING;

INSERT INTO org_memberships (org_id, user_id, role)
VALUES (
    '00000000-0000-0000-0000-000000000010',
    '00000000-0000-0000-0000-000000000001',
    'owner'
)
ON CONFLICT (org_id, user_id) DO NOTHING;

-- A fixed development key so local tooling has something stable to point at.
--   Plaintext: aegis_sk_devdevdevdevdevdevdevdevdevdevdevdevdev
-- This key is published in this file, so it is worthless anywhere but a local machine.
INSERT INTO api_keys (
    id, org_id, created_by, name, key_prefix, key_hash, rate_limit_per_minute
)
VALUES (
    '00000000-0000-0000-0000-000000000100',
    '00000000-0000-0000-0000-000000000010',
    '00000000-0000-0000-0000-000000000001',
    'development',
    'aegis_sk_devdevd',
    ENCODE(SHA256('aegis_sk_devdevdevdevdevdevdevdevdevdevdevdevdev'::BYTEA), 'hex'),
    600
)
ON CONFLICT (id) DO NOTHING;

-- ============================================================================
-- MODEL PRICING
-- ============================================================================
--
-- Superseding rather than replacing: any existing current row is closed off so the
-- history stays intact and an old invoice can still be recomputed with the prices that
-- were in force when it was issued.

UPDATE model_pricing SET effective_to = NOW() WHERE effective_to IS NULL;

INSERT INTO model_pricing (
    model_id, provider, display_name, tier,
    input_cost_per_mtok_mc, output_cost_per_mtok_mc,
    context_window, supports_tools, supports_vision, source
) VALUES
-- ---------------- OpenAI ----------------
('openai/gpt-5',            'openai', 'GPT-5',            'frontier',  1250000, 10000000,  400000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-5-mini',       'openai', 'GPT-5 mini',       'mid',        250000,  2000000,  400000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-5-nano',       'openai', 'GPT-5 nano',       'cheap',       50000,   400000,  400000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-4o',           'openai', 'GPT-4o',           'premium',   2500000, 10000000,  128000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-4o-mini',      'openai', 'GPT-4o mini',      'cheap',      150000,   600000,  128000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-4.1',          'openai', 'GPT-4.1',          'premium',   2000000,  8000000, 1047576, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-4.1-mini',     'openai', 'GPT-4.1 mini',     'mid',        400000,  1600000, 1047576, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/gpt-4.1-nano',     'openai', 'GPT-4.1 nano',     'cheap',      100000,   400000, 1047576, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/o3',               'openai', 'o3',               'frontier',  2000000,  8000000,  200000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/o4-mini',          'openai', 'o4-mini',          'premium',   1100000,  4400000,  200000, true, true, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/text-embedding-3-small', 'openai', 'Embedding 3 small', 'cheap', 20000,       0,    8191, false, false, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('openai/text-embedding-3-large', 'openai', 'Embedding 3 large', 'cheap', 130000,      0,    8191, false, false, 'OpenAI published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- Anthropic ----------------
('anthropic/claude-opus-4-5',   'anthropic', 'Claude Opus 4.5',   'frontier',  5000000, 25000000, 200000, true, true, 'Anthropic published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('anthropic/claude-sonnet-4-5', 'anthropic', 'Claude Sonnet 4.5', 'premium',   3000000, 15000000, 200000, true, true, 'Anthropic published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('anthropic/claude-haiku-4-5',  'anthropic', 'Claude Haiku 4.5',  'mid',       1000000,  5000000, 200000, true, true, 'Anthropic published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('anthropic/claude-opus-4-1',   'anthropic', 'Claude Opus 4.1',   'frontier', 15000000, 75000000, 200000, true, true, 'Anthropic published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('anthropic/claude-3-5-haiku',  'anthropic', 'Claude 3.5 Haiku',  'cheap',      800000,  4000000, 200000, true, true, 'Anthropic published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- Google ----------------
('google/gemini-2.5-pro',        'google', 'Gemini 2.5 Pro',        'premium', 1250000, 10000000, 1048576, true, true, 'Google published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('google/gemini-2.5-flash',      'google', 'Gemini 2.5 Flash',      'mid',      300000,  2500000, 1048576, true, true, 'Google published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('google/gemini-2.5-flash-lite', 'google', 'Gemini 2.5 Flash Lite', 'cheap',    100000,   400000, 1048576, true, true, 'Google published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('google/gemini-2.0-flash',      'google', 'Gemini 2.0 Flash',      'cheap',    100000,   400000, 1048576, true, true, 'Google published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- DeepSeek ----------------
('deepseek/deepseek-chat',     'deepseek', 'DeepSeek Chat',     'cheap', 270000, 1100000, 128000, true, false, 'DeepSeek published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('deepseek/deepseek-reasoner', 'deepseek', 'DeepSeek Reasoner', 'mid',   550000, 2190000, 128000, true, false, 'DeepSeek published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- Mistral ----------------
('mistral/mistral-large-latest', 'mistral', 'Mistral Large', 'premium', 2000000, 6000000, 131000, true, false, 'Mistral published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('mistral/mistral-small-latest', 'mistral', 'Mistral Small', 'cheap',    200000,  600000, 131000, true, false, 'Mistral published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- Groq ----------------
('groq/llama-3.3-70b-versatile', 'groq', 'Llama 3.3 70B (Groq)', 'mid',   590000, 790000, 131000, true, false, 'Groq published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),
('groq/llama-3.1-8b-instant',    'groq', 'Llama 3.1 8B (Groq)',  'cheap',  50000,  80000, 131000, true, false, 'Groq published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)'),

-- ---------------- Moonshot ----------------
('moonshot/kimi-k2', 'moonshot', 'Kimi K2', 'mid', 600000, 2500000, 128000, true, false, 'Moonshot published pricing — checked 2026-08-20 (UNVERIFIED, see runbook)');

-- Aliases callers actually send.
INSERT INTO model_aliases (alias, model_id) VALUES
('gpt-4o-latest',              'openai/gpt-4o'),
('chatgpt-4o-latest',          'openai/gpt-4o'),
('claude-3-5-sonnet',          'anthropic/claude-sonnet-4-5'),
('claude-sonnet-4-5-20250929', 'anthropic/claude-sonnet-4-5'),
('claude-opus-4-5-20251101',   'anthropic/claude-opus-4-5'),
('gemini-flash',               'google/gemini-2.5-flash')
ON CONFLICT (alias) DO UPDATE SET model_id = EXCLUDED.model_id;

COMMIT;

-- A visible reminder, printed at the end of the run.
DO $$
BEGIN
    RAISE NOTICE '';
    RAISE NOTICE 'Seed complete.';
    RAISE NOTICE '  Organisation: Development Org (pro plan)';
    RAISE NOTICE '  User:         dev@aegis.local';
    RAISE NOTICE '  API key:      aegis_sk_devdevdevdevdevdevdevdevdevdevdevdevdev';
    RAISE NOTICE '';
    RAISE NOTICE 'WARNING: model prices are UNVERIFIED development values.';
    RAISE NOTICE 'Run docs/runbooks/pricing-update.md before billing any customer.';
    RAISE NOTICE '';
END $$;
