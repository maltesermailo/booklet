//! The Stripe client for billing.
//!
//! Plans themselves are operator-managed rows in the database (see the `plans`
//! table and the admin Plans page) — this module only talks to Stripe. Billing is
//! **optional**: [`Stripe::from_env`] returns `None` when `STRIPE_SECRET_KEY` is
//! unset, and every billing route then reports "not configured" rather than
//! failing. We never touch card data — the operator sends a user Stripe's hosted
//! Checkout / Customer Portal URL, and a signed webhook tells us the resulting
//! plan.

use crate::auth::hex;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::env;

type HmacSha256 = Hmac<Sha256>;

/// The Stripe integration, present only when configured.
pub struct Stripe {
    secret_key: String,
    webhook_secret: String,
    public_url: String,
    http: reqwest::Client,
}

#[derive(Debug)]
pub enum Error {
    Http(reqwest::Error),
    Api(String),
    BadSignature,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(error) => write!(f, "stripe request failed: {error}"),
            Error::Api(message) => write!(f, "stripe error: {message}"),
            Error::BadSignature => write!(f, "stripe webhook signature did not verify"),
        }
    }
}

impl Stripe {
    /// Builds the client from the environment, or `None` when `STRIPE_SECRET_KEY`
    /// is unset (billing disabled). `public_url` is the panel's externally
    /// reachable base, used to build return URLs.
    pub fn from_env(public_url: &str) -> Option<Self> {
        let secret_key = env::var("STRIPE_SECRET_KEY").ok()?;
        let webhook_secret = env::var("STRIPE_WEBHOOK_SECRET").unwrap_or_default();

        Some(Self { secret_key, webhook_secret, public_url: public_url.to_string(), http: reqwest::Client::new() })
    }

    /// The hosted Checkout URL for subscribing `email` to `plan`'s price. The user
    /// pays on Stripe; the resulting subscription arrives via the webhook, which
    /// maps it back to `user_id` (client reference) and `plan` (metadata).
    pub async fn checkout_url(
        &self,
        email: &str,
        price_id: &str,
        user_id: i64,
        plan: &str,
    ) -> Result<String, Error> {
        let user_ref = user_id.to_string();
        let params = [
            ("mode", "subscription"),
            ("line_items[0][price]", price_id),
            ("line_items[0][quantity]", "1"),
            ("customer_email", email),
            ("client_reference_id", user_ref.as_str()),
            ("metadata[plan]", plan),
            ("success_url", &format!("{}/admin/users", self.public_url)),
            ("cancel_url", &format!("{}/admin/users", self.public_url)),
        ];

        self.post_for_url("https://api.stripe.com/v1/checkout/sessions", &params).await
    }

    /// The Customer Portal URL for an existing Stripe customer to manage or cancel.
    pub async fn portal_url(&self, customer_id: &str) -> Result<String, Error> {
        let params = [
            ("customer", customer_id),
            ("return_url", &format!("{}/admin/users", self.public_url)),
        ];

        self.post_for_url("https://api.stripe.com/v1/billing_portal/sessions", &params).await
    }

    async fn post_for_url(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<String, Error> {
        let response = self
            .http
            .post(endpoint)
            .basic_auth(&self.secret_key, Option::<&str>::None)
            .form(params)
            .send()
            .await
            .map_err(Error::Http)?;

        let body: serde_json::Value = response.json().await.map_err(Error::Http)?;
        if let Some(url) = body.get("url").and_then(|value| value.as_str()) {
            Ok(url.to_string())
        } else {
            Err(Error::Api(body.get("error").map(|error| error.to_string()).unwrap_or_else(|| "no url".into())))
        }
    }

    /// Verifies a `Stripe-Signature` header against the raw request body, the way
    /// Stripe documents: HMAC-SHA256 of `"{timestamp}.{body}"` under the webhook
    /// secret, compared to a `v1` scheme signature.
    pub fn verify_webhook(&self, payload: &[u8], signature_header: &str) -> Result<(), Error> {
        let (mut timestamp, mut signatures) = (None, Vec::new());
        for field in signature_header.split(',') {
            match field.split_once('=') {
                Some(("t", value)) => timestamp = Some(value),
                Some(("v1", value)) => signatures.push(value),
                _ => {}
            }
        }
        let timestamp = timestamp.ok_or(Error::BadSignature)?;

        let mut mac =
            HmacSha256::new_from_slice(self.webhook_secret.as_bytes()).map_err(|_| Error::BadSignature)?;
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(payload);
        let expected = hex(&mac.finalize().into_bytes());

        if signatures.iter().any(|candidate| constant_time_eq(candidate.as_bytes(), expected.as_bytes())) {
            Ok(())
        } else {
            Err(Error::BadSignature)
        }
    }
}

/// A length-then-content compare that does not short-circuit on the first
/// differing byte — a timing side channel is small here, but free to avoid.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripe(secret: &str) -> Stripe {
        Stripe {
            secret_key: "sk_test".into(),
            webhook_secret: secret.into(),
            public_url: "http://localhost".into(),
            http: reqwest::Client::new(),
        }
    }

    /// A signature built exactly as Stripe builds it, for the verifier to accept.
    fn sign(secret: &str, timestamp: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(body);
        format!("t={timestamp},v1={}", hex(&mac.finalize().into_bytes()))
    }

    #[test]
    fn a_webhook_signature_verifies_only_when_it_matches() {
        let stripe = stripe("whsec_secret");
        let body = br#"{"type":"checkout.session.completed"}"#;

        let good = sign("whsec_secret", "1700000000", body);
        assert!(stripe.verify_webhook(body, &good).is_ok());

        // Wrong secret, tampered body, and a malformed header all fail.
        let wrong_secret = sign("whsec_other", "1700000000", body);
        assert!(stripe.verify_webhook(body, &wrong_secret).is_err());
        assert!(stripe.verify_webhook(b"tampered", &good).is_err());
        assert!(stripe.verify_webhook(body, "garbage").is_err());
    }
}
