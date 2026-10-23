-- Friends and blocks (ADR-0021). One row per direction:
--   1 = friend, 2 = blocked (by user_id), 3 = incoming request, 4 = outgoing request.
CREATE TABLE relationships (
    user_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    peer_id    BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       SMALLINT NOT NULL CHECK (kind BETWEEN 1 AND 4),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, peer_id),
    CHECK (user_id <> peer_id)
);
CREATE INDEX relationships_peer_idx ON relationships (peer_id);
