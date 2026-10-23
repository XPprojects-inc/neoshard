-- TOTP replay protection: the last accepted 30-second time step per user.
ALTER TABLE users ADD COLUMN mfa_last_step BIGINT;
-- Federation replay nonces are only needed for the allowed clock skew.
CREATE INDEX federation_nonces_seen_idx ON federation_nonces (seen_at);
