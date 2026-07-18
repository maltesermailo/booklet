-- Display fields for the public pricing page: a shown price (in cents; NULL =
-- free / "contact us") and a short marketing description.
ALTER TABLE plans ADD COLUMN price_cents INTEGER;
ALTER TABLE plans ADD COLUMN description TEXT;

UPDATE plans SET description = 'For getting started' WHERE name = 'free';
UPDATE plans SET price_cents = 500, description = 'For a serious library' WHERE name = 'pro';
