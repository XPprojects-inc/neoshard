-- Announcement channel following: crossposts fan out to these webhooks.
CREATE TABLE channel_follows (
    source_channel_id  BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    webhook_id         BIGINT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    PRIMARY KEY (source_channel_id, webhook_id)
);
CREATE INDEX channel_follows_webhook_idx ON channel_follows (webhook_id);
