-- Migration 0008: Allow 'invite' as purpose in auth_tokens
ALTER TABLE auth_tokens DROP CONSTRAINT IF EXISTS auth_tokens_purpose_check;
ALTER TABLE auth_tokens ADD CONSTRAINT auth_tokens_purpose_check 
  CHECK (purpose::text = ANY (ARRAY['email_verify'::character varying, 'password_reset'::character varying, 'invite'::character varying]::text[]));
