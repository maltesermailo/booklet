//! A blocking HTTP client for the sync server's routes.
//!
//! One method per route, each carrying the device token. Blocking `ureq` on
//! purpose (the engine will run on a dedicated thread, per `design/sync-server.md`),
//! and status-as-error is turned off so a 409 comes back as a readable response
//! rather than an error without its body.

use booklet_sync_proto as proto;
use sha2::{Digest, Sha256};
use std::io;

/// The result of a mutation: applied, or refused with the server's current state
/// so the caller can merge.
pub enum PutResult {
    Applied(proto::PutResponse),
    Conflict(proto::Conflict),
}

/// An authenticated connection to one server.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl Client {
    /// Builds a client around a token already held (from a prior sign-in saved to
    /// disk), doing no network. Status-as-error is off so a 409 comes back as a
    /// readable response rather than an error without its body.
    pub fn with_token(base: &str, token: &str) -> Client {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().http_status_as_error(false).build().into();

        Client { agent, base: base.to_string(), token: token.to_string() }
    }

    /// Signs in and holds the issued token for subsequent calls.
    pub fn login(base: &str, request: &proto::TokenRequest) -> Result<Client, ClientError> {
        let client = Client::with_token(base, "");

        let mut response = client.agent.post(format!("{base}/auth/token")).send_json(request)?;
        expect(&response, 200)?;
        let issued: proto::TokenResponse = response.body_mut().read_json()?;

        Ok(Client { token: issued.token, ..client })
    }

    /// The device token, so the caller can persist it after sign-in.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The vaults this device's user owns, for the clone picker.
    pub fn list_vaults(&self) -> Result<Vec<proto::VaultSummary>, ClientError> {
        let mut response =
            self.agent.get(self.url("/vaults")).header("authorization", self.bearer()).call()?;
        expect(&response, 200)?;

        Ok(response.body_mut().read_json()?)
    }

    pub fn publish(&self, name: &str) -> Result<String, ClientError> {
        let mut response = self
            .agent
            .post(self.url("/vaults"))
            .header("authorization", self.bearer())
            .send_json(&proto::PublishRequest { name: name.to_string() })?;
        expect(&response, 200)?;

        let published: proto::PublishResponse = response.body_mut().read_json()?;
        Ok(published.id)
    }

    pub fn changes(&self, vault: &str, since: u64) -> Result<proto::Changes, ClientError> {
        let mut response = self
            .agent
            .get(self.url(&format!("/vaults/{vault}/changes?since={since}")))
            .header("authorization", self.bearer())
            .call()?;
        expect(&response, 200)?;

        Ok(response.body_mut().read_json()?)
    }

    /// Uploads content, returning its hash. Idempotent server-side (dedup).
    pub fn put_blob(&self, content: &[u8]) -> Result<String, ClientError> {
        let hash = sha256_hex(content);

        let response = self
            .agent
            .put(self.url(&format!("/blobs/{hash}")))
            .header("authorization", self.bearer())
            .send(content)?;
        expect(&response, 204)?;

        Ok(hash)
    }

    pub fn get_blob(&self, hash: &str) -> Result<Vec<u8>, ClientError> {
        let mut response = self
            .agent
            .get(self.url(&format!("/blobs/{hash}")))
            .header("authorization", self.bearer())
            .call()?;
        expect(&response, 200)?;

        Ok(response.body_mut().read_to_vec()?)
    }

    pub fn put_entity(
        &self,
        vault: &str,
        path: &str,
        request: &proto::PutRequest,
    ) -> Result<PutResult, ClientError> {
        let mut response = self
            .agent
            .put(self.url(&format!("/vaults/{vault}/entities/{}", encode_path(path))))
            .header("authorization", self.bearer())
            .send_json(request)?;

        put_result(&mut response)
    }

    pub fn delete_entity(&self, vault: &str, path: &str, base_version: u64) -> Result<PutResult, ClientError> {
        let mut response = self
            .agent
            .delete(self.url(&format!(
                "/vaults/{vault}/entities/{}?base_version={base_version}",
                encode_path(path)
            )))
            .header("authorization", self.bearer())
            .call()?;

        put_result(&mut response)
    }

    pub fn history(&self, vault: &str, path: &str) -> Result<proto::History, ClientError> {
        let mut response = self
            .agent
            .get(self.url(&format!("/vaults/{vault}/history/{}", encode_path(path))))
            .header("authorization", self.bearer())
            .call()?;
        expect(&response, 200)?;

        Ok(response.body_mut().read_json()?)
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.base)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

fn put_result(response: &mut ureq::http::Response<ureq::Body>) -> Result<PutResult, ClientError> {
    match response.status().as_u16() {
        200 => Ok(PutResult::Applied(response.body_mut().read_json()?)),
        409 => Ok(PutResult::Conflict(response.body_mut().read_json()?)),
        other => Err(ClientError::UnexpectedStatus(other)),
    }
}

fn expect(response: &ureq::http::Response<ureq::Body>, status: u16) -> Result<(), ClientError> {
    let actual = response.status().as_u16();
    if actual == status {
        Ok(())
    } else {
        Err(ClientError::UnexpectedStatus(actual))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Percent-encodes a vault-relative path for a URL, leaving `/` as the segment
/// separator, so a note titled `Top Note.md` or holding non-ASCII still routes.
fn encode_path(path: &str) -> String {
    path.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[derive(Debug)]
pub enum ClientError {
    Http(ureq::Error),
    Io(io::Error),
    UnexpectedStatus(u16),
    /// A merge could not be computed (from `booklet_core::merge`).
    Merge(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Http(error) => write!(f, "http error: {error}"),
            ClientError::Io(error) => write!(f, "io error: {error}"),
            ClientError::UnexpectedStatus(status) => write!(f, "unexpected status {status}"),
            ClientError::Merge(message) => write!(f, "merge error: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<ureq::Error> for ClientError {
    fn from(error: ureq::Error) -> Self {
        ClientError::Http(error)
    }
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        ClientError::Io(error)
    }
}
