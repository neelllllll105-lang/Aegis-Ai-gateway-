-- ============================================================================
-- Aegis Database Seed: Subscription Tiers & Role Architecture
-- ============================================================================
-- Free & Pro tiers: Individual Developer accounts (1 user).
-- Team & Enterprise tiers: Multi-seat Organizations with RBAC (Owner, Admin, Member, Viewer).
-- Password for all accounts: password123456
-- ============================================================================

-- 1. Organizations per Tier
INSERT INTO organizations (id, name, slug, plan, savings_share_bp, billing_email, zero_retention, content_capture, region)
VALUES 
  ('11111111-1111-1111-1111-111111111111', 'Developer Free Space', 'dev-free-space', 'free', 2000, 'dev-free@aegis.local', false, false, 'eu-central'),
  ('22222222-2222-2222-2222-222222222222', 'Developer Pro Space', 'dev-pro-space', 'pro', 2000, 'dev-pro@aegis.local', false, false, 'eu-central'),
  ('33333333-3333-3333-3333-333333333333', 'Acme Team Org', 'acme-team-org', 'team', 2000, 'billing-team@aegis.local', false, false, 'eu-central'),
  ('44444444-4444-4444-4444-444444444444', 'Acme Enterprise Corp', 'acme-enterprise-corp', 'enterprise', 2000, 'billing-enterprise@aegis.local', false, false, 'eu-central')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, plan = EXCLUDED.plan;

-- 2. Individual Users (Free & Pro Tiers - 1 Developer each)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('10000000-0000-0000-0000-000000000001', 'dev-free@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Solo Developer (Free)', false),
  ('20000000-0000-0000-0000-000000000001', 'dev-pro@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Solo Developer (Pro)', false)
ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name;

-- 3. Multi-Seat Users (Team Tier - Full RBAC)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('30000000-0000-0000-0000-000000000001', 'owner-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Owner (Billing & Admin)', false),
  ('30000000-0000-0000-0000-000000000002', 'admin-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Admin (Keys & Policies)', false),
  ('30000000-0000-0000-0000-000000000003', 'member-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Engineer (API Access)', false),
  ('30000000-0000-0000-0000-000000000004', 'viewer-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Stakeholder (Read-Only)', false)
ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name;

-- 4. Multi-Seat Users (Enterprise Tier - Full RBAC)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('40000000-0000-0000-0000-000000000001', 'owner-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise VP/Owner', false),
  ('40000000-0000-0000-0000-000000000002', 'admin-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise SecOps Admin', false),
  ('40000000-0000-0000-0000-000000000003', 'member-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise AI Engineer', false),
  ('40000000-0000-0000-0000-000000000004', 'viewer-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise Auditor (Viewer)', false)
ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name;

-- 5. Clean up previous dummy entries and map memberships cleanly
DELETE FROM org_memberships WHERE org_id IN (
  '11111111-1111-1111-1111-111111111111',
  '22222222-2222-2222-2222-222222222222',
  '33333333-3333-3333-3333-333333333333',
  '44444444-4444-4444-4444-444444444444'
);

-- Free Tier: 1 Developer (Owner of their personal space)
INSERT INTO org_memberships (org_id, user_id, role)
VALUES ('11111111-1111-1111-1111-111111111111', '10000000-0000-0000-0000-000000000001', 'owner');

-- Pro Tier: 1 Developer (Owner of their personal space)
INSERT INTO org_memberships (org_id, user_id, role)
VALUES ('22222222-2222-2222-2222-222222222222', '20000000-0000-0000-0000-000000000001', 'owner');

-- Team Tier: 4 Roles (Owner, Admin, Member, Viewer)
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000001', 'owner'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000002', 'admin'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000003', 'member'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000004', 'viewer');

-- Enterprise Tier: 4 Roles (Owner, Admin, Member, Viewer)
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000001', 'owner'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000002', 'admin'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000003', 'member'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000004', 'viewer');
