-- Development seed data.
--
--   psql "$DATABASE_URL" -f scripts/seed.sql
--
-- Creates one organisation, one owner, one API key, and the model pricing table.
--
-- # About the prices below
--
-- Every row carries a `source` naming the provider and the date it was checked, because
-- Part 13 item 8 requires every cost number to be traceable.
--
-- 28 rows were verified on 2026-08-21 against each published pricing page. Five rows
-- (mistral x2, groq x2, moonshot x1) are marked UNVERIFIED and must be confirmed before
-- they are billed on. Find them with:
--
--   SELECT model_id FROM model_pricing
--    WHERE effective_to IS NULL AND source LIKE '%UNVERIFIED%';
--
-- Re-run `docs/runbooks/pricing-update.md` monthly. A wrong number here produces a wrong
-- invoice, which Part 13 item 1 calls trust-destroying.
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
    -- A genuine argon2id hash of "aegis-development-password", produced by calling this
    -- codebase's own crypto::hash_password with that exact string — not hand-written.
    -- The value that lived here before did not verify against the password the comment
    -- claimed it hashed: it had a plausible-looking argon2id shape but
    -- was fabricated text, not a real hash of anything, so `dev@aegis.local` with the
    -- documented password had never actually been able to log in. Found live, the first
    -- time this account was used against a real database — not by inspecting the file.
    -- Development only; this account cannot exist in production because the seed is
    -- never run there.
    '$argon2id$v=19$m=19456,t=2,p=1$tRLEzu4N8hVf8/YSu877Dg$qGCbZIrgAa69kb6toeKhLxBsHnAnvo8+dCRjo37mkEo',
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

-- Prices verified 2026-08-21 against each published pricing page, except the five rows
-- marked UNVERIFIED. Micro-cents per million tokens: $2.50/Mtok = 2_500_000.
--
-- Conservative choices, applied consistently: uncached input rates, base context tier,
-- and peak rates where a provider varies price by time of day.
INSERT INTO model_pricing (
    model_id, provider, display_name, tier,
    input_cost_per_mtok_mc, output_cost_per_mtok_mc,
    context_window, supports_tools, supports_vision, is_active, source
) VALUES
-- ---------------- OpenAI (developers.openai.com/api/docs/pricing) ----------------
('openai/gpt-5',            'openai', 'GPT-5',            'frontier',  1250000, 10000000,  400000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-5-mini',       'openai', 'GPT-5 mini',       'mid',        250000,  2000000,  400000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-5-nano',       'openai', 'GPT-5 nano',       'cheap',       50000,   400000,  400000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-4o',           'openai', 'GPT-4o',           'premium',   2500000, 10000000,  128000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-4o-mini',      'openai', 'GPT-4o mini',      'cheap',      150000,   600000,  128000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-4.1',          'openai', 'GPT-4.1',          'premium',   2000000,  8000000, 1047576, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-4.1-mini',     'openai', 'GPT-4.1 mini',     'mid',        400000,  1600000, 1047576, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/gpt-4.1-nano',     'openai', 'GPT-4.1 nano',     'cheap',      100000,   400000, 1047576, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/o3',               'openai', 'o3',               'frontier',  2000000,  8000000,  200000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/o4-mini',          'openai', 'o4-mini',          'premium',   1100000,  4400000,  200000, true, true, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/text-embedding-3-small', 'openai', 'Embedding 3 small', 'cheap',  20000, 0, 8191, false, false, true, 'OpenAI published pricing — verified 2026-08-21'),
('openai/text-embedding-3-large', 'openai', 'Embedding 3 large', 'cheap', 130000, 0, 8191, false, false, true, 'OpenAI published pricing — verified 2026-08-21'),

-- ---------------- Anthropic (platform.claude.com/docs/en/about-claude/pricing) ----------------
-- Sonnet 5 at $2/$10 undercuts Sonnet 4.5 at $3/$15, so the router prefers it.
('anthropic/claude-fable-5',   'anthropic', 'Claude Fable 5',   'frontier', 10000000, 50000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-opus-5',    'anthropic', 'Claude Opus 5',    'frontier',  5000000, 25000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-opus-4-8',  'anthropic', 'Claude Opus 4.8',  'frontier',  5000000, 25000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-opus-4-7',  'anthropic', 'Claude Opus 4.7',  'frontier',  5000000, 25000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-opus-4-6',  'anthropic', 'Claude Opus 4.6',  'frontier',  5000000, 25000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-opus-4-5',  'anthropic', 'Claude Opus 4.5',  'frontier',  5000000, 25000000,  200000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-sonnet-5',  'anthropic', 'Claude Sonnet 5',  'premium',   2000000, 10000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-sonnet-4-6','anthropic', 'Claude Sonnet 4.6','premium',   3000000, 15000000, 1000000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-sonnet-4-5','anthropic', 'Claude Sonnet 4.5','premium',   3000000, 15000000,  200000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),
('anthropic/claude-haiku-4-5', 'anthropic', 'Claude Haiku 4.5', 'mid',       1000000,  5000000,  200000, true, true, true, 'Anthropic published pricing — verified 2026-08-21'),

-- ---------------- Google (ai.google.dev/gemini-api/docs/pricing, base tier) ----------------
('google/gemini-2.5-pro',        'google', 'Gemini 2.5 Pro',        'premium', 1250000, 10000000, 1048576, true, true, true, 'Google published pricing — verified 2026-08-21 (base tier; >200k input is priced higher)'),
('google/gemini-2.5-flash',      'google', 'Gemini 2.5 Flash',      'mid',      300000,  2500000, 1048576, true, true, true, 'Google published pricing — verified 2026-08-21'),
('google/gemini-2.5-flash-lite', 'google', 'Gemini 2.5 Flash Lite', 'cheap',    100000,   400000, 1048576, true, true, true, 'Google published pricing — verified 2026-08-21'),

-- ---------------- DeepSeek (api-docs.deepseek.com) — PEAK rates ----------------
('deepseek/deepseek-v4-flash', 'deepseek', 'DeepSeek V4 Flash', 'cheap',  440000, 1320000, 1000000, true, false, true, 'DeepSeek published pricing — verified 2026-08-21 (peak rate; off-peak is half)'),
('deepseek/deepseek-v4-pro',   'deepseek', 'DeepSeek V4 Pro',   'mid',   1320000, 3960000, 1000000, true, false, true, 'DeepSeek published pricing — verified 2026-08-21 (peak rate; off-peak is half)'),

-- ---------------- NOT re-verified on 2026-08-21 ----------------
('mistral/mistral-large-latest', 'mistral', 'Mistral Large', 'premium', 2000000, 6000000, 131000, true, false, true, 'UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md'),
('mistral/mistral-small-latest', 'mistral', 'Mistral Small', 'cheap',    200000,  600000, 131000, true, false, true, 'UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md'),
('groq/llama-3.3-70b-versatile', 'groq', 'Llama 3.3 70B (Groq)', 'mid',   590000, 790000, 131000, true, false, true, 'UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md'),
('groq/llama-3.1-8b-instant',    'groq', 'Llama 3.1 8B (Groq)',  'cheap',  50000,  80000, 131000, true, false, true, 'UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md'),
('moonshot/kimi-k2', 'moonshot', 'Kimi K2', 'mid', 600000, 2500000, 128000, true, false, true, 'UNVERIFIED — re-check before billing, see docs/runbooks/pricing-update.md'),

-- ---------------- Retired / deprecated: priced but never routed to ----------------
-- is_active = false keeps a historical usage record priceable while removing the model
-- from the router candidate set.
('anthropic/claude-opus-4-1',  'anthropic', 'Claude Opus 4.1 (retired)',   'frontier', 15000000, 75000000,  200000, true, true, false, 'Anthropic published pricing — verified 2026-08-21 (retired)'),
('anthropic/claude-3-5-haiku', 'anthropic', 'Claude 3.5 Haiku (retired)',  'cheap',      800000,  4000000,  200000, true, true, false, 'Anthropic published pricing — verified 2026-08-21 (retired)'),
('google/gemini-2.0-flash',    'google',    'Gemini 2.0 Flash (deprecated)','cheap',     100000,   400000, 1048576, true, true, false, 'Google published pricing — verified 2026-08-21 (deprecated)');

-- Aliases callers actually send.
INSERT INTO model_aliases (alias, model_id) VALUES
('gpt-4o-latest',              'openai/gpt-4o'),
('chatgpt-4o-latest',          'openai/gpt-4o'),
('claude-3-5-sonnet',          'anthropic/claude-sonnet-4-5'),
('claude-sonnet-4-5-20250929', 'anthropic/claude-sonnet-4-5'),
('claude-opus-4-5-20251101',   'anthropic/claude-opus-4-5'),
('gemini-flash',               'google/gemini-2.5-flash'),
('deepseek-chat',              'deepseek/deepseek-v4-flash'),
('deepseek-reasoner',          'deepseek/deepseek-v4-pro')
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
    RAISE NOTICE 'Pricing: 28 rows verified 2026-08-21; 5 still UNVERIFIED';
    RAISE NOTICE '  (mistral x2, groq x2, moonshot x1).';
    RAISE NOTICE 'Find them with:';
    RAISE NOTICE '  SELECT model_id FROM model_pricing';
    RAISE NOTICE '   WHERE effective_to IS NULL AND source LIKE ''%%UNVERIFIED%%'';';
    RAISE NOTICE '';
END $$;
