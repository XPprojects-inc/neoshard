-- Public servers that anyone on the instance can find by name and join without
-- an invite (ADR-0022). Off by default: owners opt in.
ALTER TABLE guilds
    ADD COLUMN discoverable BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN discovery_description TEXT NOT NULL DEFAULT ''
        CHECK (length(discovery_description) <= 300);

-- pg_c_utf8 (PostgreSQL 17+): Unicode case folding whatever the database locale is.
CREATE INDEX guilds_discoverable_name_idx ON guilds (lower(name COLLATE pg_c_utf8)) WHERE discoverable;
