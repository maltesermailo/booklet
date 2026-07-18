//! Outgoing email over SMTP, for password-reset links and invites.
//!
//! Optional, like billing: [`Mailer::from_env`] returns `None` unless
//! `BOOKLET_SMTP_HOST` and `BOOKLET_MAIL_FROM` are both set. With it off, the
//! panel falls back to admin-set password resets and never offers email invites.
//!
//! Configuration is discrete variables (not a credentials-in-a-URL string):
//! - `BOOKLET_SMTP_HOST` — the SMTP server hostname (required to enable email).
//! - `BOOKLET_SMTP_PORT` — port (default per TLS mode: 587 STARTTLS, 465 implicit).
//! - `BOOKLET_SMTP_USER`, `BOOKLET_SMTP_PASSWORD` — login, if the server needs one.
//! - `BOOKLET_SMTP_TLS` — `starttls` (default), `implicit`, or `none` (dev only).
//! - `BOOKLET_MAIL_FROM` — the From address (`Name <addr>` accepted; required).

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{AsyncTransport, Message, Tokio1Executor};
use std::env;

/// A configured SMTP sender plus the public base URL that links in emails point at.
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    public_url: String,
}

impl Mailer {
    /// Builds the mailer from the discrete `BOOKLET_SMTP_*` / `BOOKLET_MAIL_FROM`
    /// variables (see the module docs); `None` when email is not configured.
    pub fn from_env() -> Option<Self> {
        let host = env::var("BOOKLET_SMTP_HOST").ok()?;
        let from: Mailbox = env::var("BOOKLET_MAIL_FROM").ok()?.parse().ok()?;
        let public_url = env::var("BOOKLET_PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:8081".into());

        // STARTTLS on 587 by default; implicit TLS on 465; or plaintext for a
        // local dev relay.
        let mut builder = match env::var("BOOKLET_SMTP_TLS").as_deref() {
            Ok("none") => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host),
            Ok("implicit") => AsyncSmtpTransport::<Tokio1Executor>::relay(&host).ok()?,
            _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host).ok()?,
        };

        if let Some(port) = env::var("BOOKLET_SMTP_PORT").ok().and_then(|port| port.parse::<u16>().ok()) {
            builder = builder.port(port);
        }
        if let Ok(user) = env::var("BOOKLET_SMTP_USER") {
            let password = env::var("BOOKLET_SMTP_PASSWORD").unwrap_or_default();
            builder = builder.credentials(Credentials::new(user, password));
        }

        Some(Self { transport: builder.build(), from, public_url })
    }

    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    /// Emails a password-reset link carrying the one-time token.
    pub async fn send_reset(&self, to: &str, token: &str) -> Result<(), String> {
        let link = format!("{}/admin/reset/{token}", self.public_url);
        let body = format!(
            "Someone asked to reset your Booklet password.\n\nOpen this link to set a new one (it expires in an hour):\n{link}\n\nIf it wasn't you, ignore this email."
        );

        self.send(to, "Reset your Booklet password", body).await
    }

    /// Emails an invite link carrying the registration token.
    pub async fn send_invite(&self, to: &str, token: &str) -> Result<(), String> {
        let link = format!("{}/signup?invite={token}", self.public_url);
        let body = format!("You've been invited to Booklet.\n\nCreate your account here:\n{link}\n");

        self.send(to, "You're invited to Booklet", body).await
    }

    async fn send(&self, to: &str, subject: &str, body: String) -> Result<(), String> {
        let recipient: Mailbox = to.parse().map_err(|_| format!("invalid email address: {to}"))?;
        let email = Message::builder()
            .from(self.from.clone())
            .to(recipient)
            .subject(subject)
            .body(body)
            .map_err(|error| error.to_string())?;

        self.transport.send(email).await.map(|_| ()).map_err(|error| error.to_string())
    }
}
