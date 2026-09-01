-- Seed Organizations for each Tier
INSERT INTO organizations (id, name, slug, plan, savings_share_bp, billing_email, zero_retention, content_capture, region)
VALUES 
  ('11111111-1111-1111-1111-111111111111', 'Acme Free', 'acme-free', 'free', 2000, 'billing-free@aegis.local', false, false, 'eu-central'),
  ('22222222-2222-2222-2222-222222222222', 'Acme Pro', 'acme-pro', 'pro', 2000, 'billing-pro@aegis.local', false, false, 'eu-central'),
  ('33333333-3333-3333-3333-333333333333', 'Acme Team', 'acme-team', 'team', 2000, 'billing-team@aegis.local', false, false, 'eu-central'),
  ('44444444-4444-4444-4444-444444444444', 'Acme Enterprise', 'acme-enterprise', 'enterprise', 2000, 'billing-enterprise@aegis.local', false, false, 'eu-central')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, plan = EXCLUDED.plan;

-- Seed Users for Free Tier (owner, admin, member, viewer)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('10000000-0000-0000-0000-000000000001', 'owner-free@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Free Tier Owner', false),
  ('10000000-0000-0000-0000-000000000002', 'admin-free@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Free Tier Admin', false),
  ('10000000-0000-0000-0000-000000000003', 'member-free@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Free Tier Member', false),
  ('10000000-0000-0000-0000-000000000004', 'viewer-free@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Free Tier Viewer', false)
ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name;

-- Seed Users for Pro Tier (owner, admin, member, viewer)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('20000000-0000-0000-0000-000000000001', 'owner-pro@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Pro Tier Owner', false),
  ('20000000-0000-0000-0000-000000000002', 'admin-pro@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Pro Tier Admin', false),
  ('20000000-0000-0000-0000-000000000003', 'member-pro@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Pro Tier Member', false),
  ('20000000-0000-0000-0000-000000000004', 'viewer-pro@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Pro Tier Viewer', false)
ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name;

-- Seed Users for Team Tier (owner, admin, member, viewer)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('30000000-0000-0000-0000-000000000001', 'owner-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Tier Owner', false),
  ('30000000-0000-0000-0000-000000000002', 'admin-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Tier Admin', false),
  ('30000000-0000-0000-0000-000000000003', 'member-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Tier Member', false),
  ('30000000-0000-0000-0000-000000000004', 'viewer-team@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Team Tier Viewer', false)
ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name;

-- Seed Users for Enterprise Tier (owner, admin, member, viewer)
INSERT INTO users (id, email, email_verified_at, password_hash, name, is_admin)
VALUES 
  ('40000000-0000-0000-0000-000000000001', 'owner-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise Tier Owner', false),
  ('40000000-0000-0000-0000-000000000002', 'admin-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise Tier Admin', false),
  ('40000000-0000-0000-0000-000000000003', 'member-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise Tier Member', false),
  ('40000000-0000-0000-0000-000000000004', 'viewer-enterprise@aegis.local', NOW(), '$argon2id$v=19$m=19456,t=2,p=1$wOr5Z7+FZKkctsBbQl8zpw$6hBSkrjmCvvWu3RSJ8zZz4YhWi8c2RyNj0B+eBZLr8A', 'Enterprise Tier Viewer', false)
ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name;

-- Link Memberships for Free Tier
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('11111111-1111-1111-1111-111111111111', '10000000-0000-0000-0000-000000000001', 'owner'),
  ('11111111-1111-1111-1111-111111111111', '10000000-0000-0000-0000-000000000002', 'admin'),
  ('11111111-1111-1111-1111-111111111111', '10000000-0000-0000-0000-000000000003', 'member'),
  ('11111111-1111-1111-1111-111111111111', '10000000-0000-0000-0000-000000000004', 'viewer')
ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role;

-- Link Memberships for Pro Tier
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('22222222-2222-2222-2222-222222222222', '20000000-0000-0000-0000-000000000001', 'owner'),
  ('22222222-2222-2222-2222-222222222222', '20000000-0000-0000-0000-000000000002', 'admin'),
  ('22222222-2222-2222-2222-222222222222', '20000000-0000-0000-0000-000000000003', 'member'),
  ('22222222-2222-2222-2222-222222222222', '20000000-0000-0000-0000-000000000004', 'viewer')
ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role;

-- Link Memberships for Team Tier
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000001', 'owner'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000002', 'admin'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000003', 'member'),
  ('33333333-3333-3333-3333-333333333333', '30000000-0000-0000-0000-000000000004', 'viewer')
ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role;

-- Link Memberships for Enterprise Tier
INSERT INTO org_memberships (org_id, user_id, role)
VALUES 
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000001', 'owner'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000002', 'admin'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000003', 'member'),
  ('44444444-4444-4444-4444-444444444444', '40000000-0000-0000-0000-000000000004', 'viewer')
ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role;
