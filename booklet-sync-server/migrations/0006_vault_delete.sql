-- Soft-delete for vaults: a deleted vault vanishes from the user's list and all
-- its sync routes, but its rows and blobs are retained as a recoverable backup
-- (nothing is destroyed on delete). An operator can still see it in the panel.
ALTER TABLE vaults ADD COLUMN deleted_at TIMESTAMPTZ;
