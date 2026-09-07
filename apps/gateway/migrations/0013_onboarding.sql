-- Migration 0013: onboarding walkthrough completion.
--
-- Per-user, not per-org: this tracks whether *this person* has seen the guided tour, so a
-- new admin added to an existing Team/Enterprise org still gets it, and one owner replaying
-- it from Settings does not mark it seen for every other member of the organisation.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS onboarding_completed_at TIMESTAMPTZ NULL;
