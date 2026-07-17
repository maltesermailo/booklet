-- Quotas, billing, and email-driven account flows. See ROADMAP.md M7 (the
-- deferred features) — built on the user's request atop the admin panel.

-- An optional email, needed by the email flows and billing (the account model
-- was handle-only until now). UNIQUE when present, so "forgot password" and
-- invites resolve to one account; multiple NULLs are allowed.
ALTER TABLE users ADD COLUMN email               TEXT UNIQUE;

-- Per-user quota and subscription. quota_bytes is a manual admin override; when
-- NULL the plan's default applies.
ALTER TABLE users ADD COLUMN quota_bytes         BIGINT;
ALTER TABLE users ADD COLUMN plan                TEXT NOT NULL DEFAULT 'free';
ALTER TABLE users ADD COLUMN stripe_customer_id  TEXT;
ALTER TABLE users ADD COLUMN subscription_status TEXT;

-- Single-use, expiring password-reset tokens (email flow). Only the hash is kept.
CREATE TABLE password_resets (
  id         BIGSERIAL PRIMARY KEY,
  user_id    BIGINT NOT NULL REFERENCES users(id),
  token_hash TEXT UNIQUE NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  used       BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Admin-issued invites: a token mailed to an address, redeemed at registration.
CREATE TABLE invites (
  id         BIGSERIAL PRIMARY KEY,
  token_hash TEXT UNIQUE NOT NULL,
  email      TEXT NOT NULL,
  invited_by BIGINT NOT NULL REFERENCES users(id),
  expires_at TIMESTAMPTZ NOT NULL,
  accepted   BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
