-- Operator-managed custom plans, replacing the two hardcoded tiers and the
-- BOOKLET_PLAN_*_GIB env overrides. A plan is a row an admin creates in the
-- panel; users.plan references one by name, loosely (like the theme name) — a
-- missing plan means "no quota", never an error.

CREATE TABLE plans (
  name            TEXT PRIMARY KEY,
  quota_bytes     BIGINT NOT NULL,
  stripe_price_id TEXT,                              -- NULL = not sold via Stripe
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Seed the two former built-in tiers so existing accounts keep their quota. The
-- operator can rename, retune, delete, or add plans from here on.
INSERT INTO plans (name, quota_bytes) VALUES
  ('free', 1073741824),                              -- 1 GiB
  ('pro',  53687091200);                             -- 50 GiB
