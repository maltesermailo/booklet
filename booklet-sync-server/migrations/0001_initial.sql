-- Booklet sync server schema. See design/sync-server.md.
-- PostgreSQL 13+ (gen_random_uuid() is in core).

CREATE TABLE users (
  id            BIGSERIAL PRIMARY KEY,
  handle        TEXT UNIQUE NOT NULL,
  password_hash TEXT NOT NULL,                    -- argon2id
  is_admin      BOOLEAN NOT NULL DEFAULT FALSE,
  disabled      BOOLEAN NOT NULL DEFAULT FALSE,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at  TIMESTAMPTZ
);

CREATE TABLE devices (
  id           BIGSERIAL PRIMARY KEY,
  user_id      BIGINT NOT NULL REFERENCES users(id),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE NOT NULL,              -- sha-256(token)
  issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at TIMESTAMPTZ,
  revoked_at   TIMESTAMPTZ
);

CREATE TABLE vaults (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    BIGINT NOT NULL REFERENCES users(id),
  name       TEXT NOT NULL,
  seq        BIGINT NOT NULL DEFAULT 0,           -- monotonic per-vault sequence
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Blobs are content-addressed; their bytes on disk are a full zstd checkpoint or
-- a delta against base_hash. Chain metadata lives here, the source of truth the
-- store reads back to reconstruct.
CREATE TABLE blobs (
  hash        TEXT PRIMARY KEY,                   -- sha-256 hex of the original content
  size        BIGINT NOT NULL,                    -- original content length
  stored_size BIGINT NOT NULL,                    -- bytes on disk
  encoding    TEXT NOT NULL,                      -- 'full' | 'delta'
  base_hash   TEXT REFERENCES blobs(hash),
  depth       INTEGER NOT NULL DEFAULT 0,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Current state, one row per path. GET /changes scans this by seq.
CREATE TABLE entities (
  vault_id   UUID NOT NULL REFERENCES vaults(id),
  path       TEXT NOT NULL,
  kind       TEXT NOT NULL,                       -- 'note' | 'bookmeta' | 'folder' | 'image'
  version    BIGINT NOT NULL,
  seq        BIGINT NOT NULL,
  blob       TEXT REFERENCES blobs(hash),         -- content hash; NULL for folders and deletes
  deleted    BOOLEAN NOT NULL DEFAULT FALSE,
  moved_from TEXT,
  PRIMARY KEY (vault_id, path)
);
CREATE INDEX entities_feed ON entities(vault_id, seq);

-- History, forever: the base for the next merge and the recovery path for a bad one.
CREATE TABLE entity_versions (
  vault_id   UUID NOT NULL,
  path       TEXT NOT NULL,
  version    BIGINT NOT NULL,
  seq        BIGINT NOT NULL,
  blob       TEXT REFERENCES blobs(hash),
  deleted    BOOLEAN NOT NULL DEFAULT FALSE,
  moved_from TEXT,
  device_id  BIGINT REFERENCES devices(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (vault_id, path, version)
);
