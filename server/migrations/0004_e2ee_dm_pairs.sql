-- A user pair may have one plain and one end-to-end encrypted 1:1 DM (ADR-0013).
ALTER TABLE dm_pairs ADD COLUMN e2ee BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE dm_pairs DROP CONSTRAINT dm_pairs_pkey;
ALTER TABLE dm_pairs ADD PRIMARY KEY (user_a, user_b, e2ee);
