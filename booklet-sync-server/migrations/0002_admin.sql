-- Admin panel: sessions and the audit log. See ROADMAP.md M7.
-- The users.is_admin flag already exists (0001_initial.sql); M7 only reads it.

-- An admin session is a separate credential from a device token: a signed-in
-- operator, not a synced laptop. Opaque high-entropy id, sha-256 stored, so
-- revocation is a row delete. csrf is this session's synchronizer token.
CREATE TABLE admin_sessions (
  id           BIGSERIAL PRIMARY KEY,
  user_id      BIGINT NOT NULL REFERENCES users(id),
  token_hash   TEXT UNIQUE NOT NULL,               -- sha-256(session id)
  csrf         TEXT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at   TIMESTAMPTZ NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Append-only operations log: sign-ins, token issues/revocations, user and
-- vault mutations. Capped in the read, plain rows.
CREATE TABLE audit_log (
  id     BIGSERIAL PRIMARY KEY,
  at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  actor  TEXT NOT NULL,                             -- admin handle, or '' for the system
  action TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT ''
);
CREATE INDEX audit_log_recent ON audit_log(at DESC);
