-- Profile customization (ADR-0020): everything is free on NeoShard, nothing is paywalled.
-- Catalog ids (decorations, effects, nameplates, fonts) are validated in code
-- against neoshard_proto::profile; their visuals live in the client.

ALTER TABLE users
    ADD COLUMN pronouns            TEXT NOT NULL DEFAULT '' CHECK (length(pronouns) <= 40),
    ADD COLUMN bio                 TEXT NOT NULL DEFAULT '' CHECK (length(bio) <= 400),
    ADD COLUMN banner              TEXT NOT NULL DEFAULT '',
    ADD COLUMN banner_color        INTEGER CHECK (banner_color BETWEEN 0 AND 16777215),
    ADD COLUMN theme_colors        INTEGER[] NOT NULL DEFAULT '{}' CHECK (cardinality(theme_colors) IN (0, 2)),
    ADD COLUMN avatar_decoration   TEXT NOT NULL DEFAULT '',
    ADD COLUMN profile_effect      TEXT NOT NULL DEFAULT '',
    ADD COLUMN nameplate           TEXT NOT NULL DEFAULT '',
    ADD COLUMN name_font           TEXT NOT NULL DEFAULT '',
    ADD COLUMN name_effect         TEXT NOT NULL DEFAULT '',
    ADD COLUMN name_colors         INTEGER[] NOT NULL DEFAULT '{}' CHECK (cardinality(name_colors) <= 2),
    ADD COLUMN custom_status_emoji TEXT NOT NULL DEFAULT '' CHECK (length(custom_status_emoji) <= 64),
    ADD COLUMN custom_status_expires_at TIMESTAMPTZ,
    ADD COLUMN primary_guild_id    BIGINT REFERENCES guilds(id) ON DELETE SET NULL;

-- Per-server profile: empty / NULL falls back to the user profile.
ALTER TABLE members
    ADD COLUMN avatar        TEXT NOT NULL DEFAULT '',
    ADD COLUMN banner        TEXT NOT NULL DEFAULT '',
    ADD COLUMN bio           TEXT NOT NULL DEFAULT '' CHECK (length(bio) <= 400),
    ADD COLUMN pronouns      TEXT NOT NULL DEFAULT '' CHECK (length(pronouns) <= 40),
    ADD COLUMN theme_colors  INTEGER[] NOT NULL DEFAULT '{}' CHECK (cardinality(theme_colors) IN (0, 2));

-- Server tag that members may show next to their name.
ALTER TABLE guilds
    ADD COLUMN tag        TEXT NOT NULL DEFAULT '' CHECK (length(tag) <= 4),
    ADD COLUMN tag_badge  TEXT NOT NULL DEFAULT '';

-- Private notes about other users (visible only to their author).
CREATE TABLE user_notes (
    owner_id   BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    note       TEXT NOT NULL CHECK (length(note) BETWEEN 1 AND 256),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner_id, target_id)
);

-- Profile links (the open counterpart of "connections"): verified by a rel="me"
-- backlink to the user's profile page, no third-party OAuth apps needed.
CREATE TABLE user_links (
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    position    SMALLINT NOT NULL CHECK (position BETWEEN 0 AND 5),
    label       TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 32),
    url         TEXT NOT NULL CHECK (length(url) <= 512 AND url ~ '^https?://'),
    verified_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, position)
);
