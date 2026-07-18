//! qtbridge adapter hosting the background sync engine.
//!
//! The blocking `booklet-sync-client` engine runs on a dedicated `std::thread`
//! (mirroring the file watcher in `library.rs`): the UI sends it commands over a
//! channel, and it reports back by depositing events into a shared `Mutex` and
//! calling `pump` through a `QmlMethodInvoker` — the only safe way onto the Qt
//! thread from a foreign one. The engine, its `Client`, and its per-vault state
//! all live on that thread and are never shared with the Qt-confined `Engine`.
//!
//! Signals carry their full payload so a QML handler never has to call back into
//! this object while a slot is mid-emit (the `Rc<RefCell>` re-entrancy rule).

use booklet_core::sync::{Manifest, SyncState};
use booklet_sync_client::{engine, secret, Client, ClientError, ClientState, Credentials};
use booklet_sync_proto as proto;
use qtbridge::{qobject, QObjectHolder, QmlMethodInvoker};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const POLL: Duration = Duration::from_secs(30);

pub struct Sync {
    tx: Option<Sender<Command>>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Mutex<Shared>>,
}

impl Default for Sync {
    fn default() -> Self {
        Self { tx: None, thread: None, shared: Arc::new(Mutex::new(Shared::default())) }
    }
}

#[qobject(Singleton)]
impl Sync {
    /// Spawns the sync thread. Call once at startup.
    #[qslot]
    fn start(&mut self) {
        if self.thread.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel();
        let worker = Worker::new(self.get_qml_method_invoker(), Arc::clone(&self.shared));

        self.tx = Some(tx);
        self.thread = Some(std::thread::spawn(move || worker.run(rx)));
    }

    /// `{ server, handle, password, device }` — trades credentials for a device
    /// token, saved to its own file.
    #[qslot]
    fn sign_in(&mut self, payload: String) {
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        self.send(Command::SignIn {
            server: string(&value, "server"),
            handle: string(&value, "handle"),
            password: string(&value, "password"),
            device: string(&value, "device"),
        });
    }

    #[qslot]
    fn sign_out(&mut self) {
        self.send(Command::SignOut);
    }

    /// The active vault changed; scope sync to it.
    #[qslot]
    fn set_active_vault(&mut self, path: String) {
        self.send(Command::SetVault(PathBuf::from(path)));
    }

    #[qslot]
    fn sync_now(&mut self) {
        self.send(Command::Sync);
    }

    /// Binds the active vault to a new server vault named `name` and pushes it.
    #[qslot]
    fn publish(&mut self, name: String) {
        self.send(Command::Publish(name));
    }

    /// Deletes the active vault from the server. The server keeps the data as a
    /// backup and every local file stays on disk; the vault becomes local-only.
    #[qslot]
    fn delete_vault(&mut self) {
        self.send(Command::DeleteVault);
    }

    /// `{ vault_id, path }` — clones a server vault into the empty local folder.
    #[qslot]
    fn clone_vault(&mut self, payload: String) {
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        self.send(Command::Clone {
            vault_id: string(&value, "vault_id"),
            root: PathBuf::from(string(&value, "path")),
        });
    }

    /// Asks the server for the user's vaults; answered by `vaults_ready`.
    #[qslot]
    fn request_vaults(&mut self) {
        self.send(Command::RequestVaults);
    }

    /// Asks for a note's version history; answered by `history_ready`.
    #[qslot]
    fn request_history(&mut self, path: String) {
        self.send(Command::RequestHistory(path));
    }

    /// `{ path, version }` — fetches an old version's text; answered by
    /// `version_ready` for the history modal to show.
    #[qslot]
    fn request_version(&mut self, payload: String) {
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        self.send(Command::RequestVersion {
            path: string(&value, "path"),
            version: value.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0),
        });
    }

    /// `{ path, version }` — restores an old version as a fresh local edit.
    #[qslot]
    fn restore(&mut self, payload: String) {
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        self.send(Command::Restore {
            path: string(&value, "path"),
            version: value.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0),
        });
    }

    /// The user reviewed a flagged note (`path` is absolute); clear the flag.
    #[qslot]
    fn dismiss_flag(&mut self, path: String) {
        self.send(Command::DismissFlag(path));
    }

    /// The current status as `{ state, flagged_count, signed_in, published }`,
    /// for the pill and the sync menu on load.
    #[qslot]
    fn status(&self) -> String {
        let shared = self.shared.lock().unwrap();
        status_json(&shared.status, shared.flagged.len(), shared.signed_in, shared.published)
    }

    /// Whether the active vault is already bound to a server vault, so the UI can
    /// hide "Publish" once it is.
    #[qslot]
    fn is_published(&self) -> bool {
        self.shared.lock().unwrap().published
    }

    /// Whether the note at absolute `path` is flagged, for the editor on open.
    #[qslot]
    fn is_flagged(&self, path: String) -> bool {
        self.shared.lock().unwrap().flagged.iter().any(|flagged| flagged == &path)
    }

    /// Whether a device token is held, for the Settings sync pane.
    #[qslot]
    fn is_signed_in(&self) -> bool {
        self.shared.lock().unwrap().signed_in
    }

    /// Drains the events the worker deposited and emits them. Invoked by the
    /// worker thread through the `QmlMethodInvoker`.
    #[qslot]
    fn pump(&mut self) {
        let (status, flagged_count, signed_in, published, events) = {
            let mut shared = self.shared.lock().unwrap();
            let events = std::mem::take(&mut shared.events);
            (shared.status.clone(), shared.flagged.len(), shared.signed_in, shared.published, events)
        };

        for event in events {
            match event {
                Event::Status => self.status_changed(status_json(&status, flagged_count, signed_in, published)),
                Event::NoteChanged(id) => self.note_changed(id),
                Event::SignedIn(ok) => self.signed_in(ok),
                Event::VaultsReady(json) => self.vaults_ready(json),
                Event::HistoryReady(json) => self.history_ready(json),
                Event::VersionReady(json) => self.version_ready(json),
                Event::Notice(message) => self.notice(message),
                Event::Failed(message) => self.failed(message),
            }
        }
    }

    /// `{ state, flagged_count }` — the pill re-reads it.
    #[qsignal]
    fn status_changed(&mut self, payload: String);

    /// A note's file changed on disk (absolute path); the editor reloads it if
    /// it is the open one.
    #[qsignal]
    fn note_changed(&mut self, id: String);

    #[qsignal]
    fn signed_in(&mut self, ok: bool);

    /// A positive confirmation the user should see (sign-in, publish).
    #[qsignal]
    fn notice(&mut self, message: String);

    /// The user's server vaults as JSON, for the clone picker.
    #[qsignal]
    fn vaults_ready(&mut self, payload: String);

    /// A note's version history as JSON, for the version modal.
    #[qsignal]
    fn history_ready(&mut self, payload: String);

    /// `{ version, content }` — an old version's text, for the modal to show.
    #[qsignal]
    fn version_ready(&mut self, payload: String);

    #[qsignal]
    fn failed(&mut self, message: String);
}

impl Sync {
    fn send(&mut self, command: Command) {
        if let Some(tx) = &self.tx {
            if tx.send(command).is_err() {
                self.failed("The sync engine has stopped.".to_string());
            }
        }
    }
}

// --- the worker thread ---

enum Command {
    SignIn { server: String, handle: String, password: String, device: String },
    SignOut,
    SetVault(PathBuf),
    Sync,
    Publish(String),
    DeleteVault,
    Clone { vault_id: String, root: PathBuf },
    RequestVaults,
    RequestHistory(String),
    RequestVersion { path: String, version: u64 },
    Restore { path: String, version: u64 },
    DismissFlag(String),
}

enum Event {
    Status,
    NoteChanged(String),
    SignedIn(bool),
    VaultsReady(String),
    HistoryReady(String),
    VersionReady(String),
    /// A positive confirmation for the user (e.g. "Published …").
    Notice(String),
    Failed(String),
}

#[derive(Default)]
struct Shared {
    status: String,
    signed_in: bool,
    /// Whether the active vault is bound to a server vault (already published or
    /// cloned). Drives the "Publish" affordance so a vault is never published
    /// twice.
    published: bool,
    /// Flagged notes as **absolute** paths (what the editor and pill compare).
    flagged: Vec<String>,
    events: Vec<Event>,
}

/// The active vault's sync state on the worker thread.
struct VaultSync {
    root: PathBuf,
    state: SyncState,
    manifest: Manifest,
}

struct Worker {
    invoker: QmlMethodInvoker,
    shared: Arc<Mutex<Shared>>,
    token_path: PathBuf,
    credentials: Option<Credentials>,
    client: Option<Client>,
    vault: Option<VaultSync>,
}

impl Worker {
    fn new(invoker: QmlMethodInvoker, shared: Arc<Mutex<Shared>>) -> Worker {
        let token_path = token_path();
        let credentials = secret::load(&token_path).ok().flatten();
        let client = credentials.as_ref().map(|c| Client::with_token(&c.server_url, &c.token));

        Worker { invoker, shared, token_path, credentials, client, vault: None }
    }

    fn run(mut self, rx: mpsc::Receiver<Command>) {
        self.set_status("offline");
        self.set_signed_in(self.credentials.is_some());

        loop {
            let command = match rx.recv_timeout(POLL) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => Command::Sync,
                Err(RecvTimeoutError::Disconnected) => break,
            };

            match command {
                Command::SignIn { server, handle, password, device } => {
                    self.sign_in(server, handle, password, device)
                }
                Command::SignOut => self.sign_out(),
                Command::SetVault(root) => self.set_vault(root),
                Command::Sync => self.sync(),
                Command::Publish(name) => self.publish(&name),
                Command::DeleteVault => self.delete_server_vault(),
                Command::Clone { vault_id, root } => self.clone_vault(vault_id, root),
                Command::RequestVaults => self.request_vaults(),
                Command::RequestHistory(path) => self.request_history(&path),
                Command::RequestVersion { path, version } => self.request_version(&path, version),
                Command::Restore { path, version } => self.restore(&path, version),
                Command::DismissFlag(path) => self.dismiss_flag(&path),
            }
        }
    }

    fn sign_in(&mut self, server: String, handle: String, password: String, device: String) {
        let request = proto::TokenRequest {
            handle: handle.clone(),
            password,
            device_name: device,
            platform: std::env::consts::OS.to_string(),
        };

        match Client::login(&server, &request) {
            Ok(client) => {
                let credentials =
                    Credentials { server_url: server, handle, token: client.token().to_string() };
                if let Err(error) = secret::save(&self.token_path, &credentials) {
                    return self.fail(&format!("Could not save the sign-in: {error}"));
                }

                self.credentials = Some(credentials);
                self.client = Some(client);
                self.set_status("offline");
                self.set_signed_in(true);
                self.commit(vec![Event::Status, Event::SignedIn(true), Event::Notice("Signed in.".into())]);
                self.sync();
            }
            Err(error) => {
                self.commit(vec![Event::SignedIn(false), Event::Failed(format!("Could not sign in: {error}"))]);
            }
        }
    }

    fn sign_out(&mut self) {
        let _ = secret::clear(&self.token_path);
        self.credentials = None;
        self.client = None;
        self.set_status("offline");
        self.set_signed_in(false);
        self.commit(vec![Event::Status, Event::SignedIn(false)]);
    }

    fn set_vault(&mut self, root: PathBuf) {
        let state = SyncState::load(&root).unwrap_or_default();
        let manifest = Manifest::load(&root).unwrap_or_default();
        let bound = state.vault_id.is_some();

        self.set_flagged_from(&root, &state.flagged);
        self.set_published(bound);
        self.vault = Some(VaultSync { root, state, manifest });
        self.set_status("offline");
        self.commit(vec![Event::Status]);

        if bound && self.client.is_some() {
            self.sync();
        }
    }

    fn sync(&mut self) {
        let Some(client) = self.client.clone() else { return };
        let vault_id = match self.vault.as_ref().and_then(|vault| vault.state.vault_id.clone()) {
            Some(id) => id,
            None => return,
        };

        self.set_status("syncing");
        self.commit(vec![Event::Status]);
        let today = today();

        let outcome = {
            let vault = self.vault.as_mut().unwrap();
            reconcile(&client, &vault_id, vault, &today)
        };

        match outcome {
            Ok((notes, flagged, root)) => {
                self.set_flagged_from(&root, &flagged);
                self.set_status("synced");
                let mut events = vec![Event::Status];
                events.extend(notes.into_iter().map(|id| Event::NoteChanged(absolute(&root, &id))));
                self.commit(events);
            }
            Err(error) => self.report(error),
        }
    }

    fn publish(&mut self, name: &str) {
        let Some(client) = self.client.clone() else {
            return self.fail("Sign in before publishing.");
        };
        // A vault is published exactly once: re-publishing would create a second,
        // duplicate server vault. The worker processes commands serially, so this
        // guard also catches a rapid double-click.
        match self.vault.as_ref().map(|vault| vault.state.vault_id.is_some()) {
            None => return self.fail("Open a vault before publishing."),
            Some(true) => return self.fail("This vault is already published."),
            Some(false) => {}
        }
        let server_url = self.credentials.as_ref().map(|c| c.server_url.clone());

        match client.publish(name) {
            Ok(vault_id) => {
                let vault = self.vault.as_mut().unwrap();
                vault.state.vault_id = Some(vault_id);
                vault.state.server_url = server_url;
                vault.manifest = Manifest::default(); // force a full push of everything
                let _ = vault.state.save(&vault.root);
                self.set_published(true);
                self.commit(vec![Event::Notice(format!("Published “{name}”."))]);
                self.sync();
            }
            Err(error) => self.fail(&format!("Could not publish: {error}")),
        }
    }

    fn delete_server_vault(&mut self) {
        let Some(client) = self.client.clone() else {
            return self.fail("Sign in before deleting.");
        };
        let Some(vault_id) = self.vault.as_ref().and_then(|vault| vault.state.vault_id.clone()) else {
            return self.fail("This vault isn't published.");
        };

        match client.delete_vault(&vault_id) {
            Ok(()) => {
                // Unbind locally but keep every markdown file — the local backup.
                let vault = self.vault.as_mut().unwrap();
                vault.state = SyncState::default();
                vault.manifest = Manifest::default();
                let _ = vault.state.save(&vault.root);
                let root = vault.root.clone();

                self.set_published(false);
                self.set_flagged_from(&root, &[]);
                self.set_status("offline");
                self.commit(vec![
                    Event::Status,
                    Event::Notice("Vault deleted from the server. Your local files are kept.".into()),
                ]);
            }
            Err(error) => self.fail(&format!("Could not delete the vault: {error}")),
        }
    }

    fn clone_vault(&mut self, vault_id: String, root: PathBuf) {
        let server_url = self.credentials.as_ref().map(|c| c.server_url.clone());
        let state = SyncState {
            vault_id: Some(vault_id),
            server_url,
            ..SyncState::default()
        };
        if let Err(error) = state.save(&root) {
            return self.fail(&format!("Could not bind the vault: {error}"));
        }

        self.vault = Some(VaultSync { root, state, manifest: Manifest::default() });
        self.set_published(true);
        self.sync();
    }

    fn request_vaults(&mut self) {
        let Some(client) = self.client.clone() else { return };
        match client.list_vaults() {
            Ok(vaults) => {
                let json = serde_json::to_string(&vaults).expect("vaults serialize to JSON");
                self.commit(vec![Event::VaultsReady(json)]);
            }
            Err(error) => self.report(error),
        }
    }

    fn request_history(&mut self, path: &str) {
        let Some((client, vault_id)) = self.bound_client() else { return };
        match client.history(&vault_id, path) {
            Ok(history) => {
                let json = serde_json::to_string(&history.versions).expect("history serializes to JSON");
                self.commit(vec![Event::HistoryReady(json)]);
            }
            Err(error) => self.report(error),
        }
    }

    fn request_version(&mut self, path: &str, version: u64) {
        let Some((client, vault_id)) = self.bound_client() else { return };
        match restore_content(&client, &vault_id, path, version) {
            Ok(content) => {
                let json = serde_json::json!({
                    "version": version,
                    "content": String::from_utf8_lossy(&content),
                })
                .to_string();
                self.commit(vec![Event::VersionReady(json)]);
            }
            Err(error) => self.report(error),
        }
    }

    fn restore(&mut self, path: &str, version: u64) {
        let Some((client, vault_id)) = self.bound_client() else { return };
        let Some(root) = self.vault.as_ref().map(|vault| vault.root.clone()) else { return };

        let restored = restore_content(&client, &vault_id, path, version);
        match restored {
            Ok(content) => {
                if let Err(error) = std::fs::write(root.join(path), content) {
                    return self.fail(&format!("Could not restore: {error}"));
                }
                self.commit(vec![Event::NoteChanged(absolute(&root, path))]);
                self.sync(); // push the restored content
            }
            Err(error) => self.report(error),
        }
    }

    fn dismiss_flag(&mut self, absolute_path: &str) {
        let Some(vault) = self.vault.as_mut() else { return };
        let relative = relative(&vault.root, absolute_path);

        vault.state.flagged.retain(|flagged| flagged != &relative);
        let _ = vault.state.save(&vault.root);
        let flagged = vault.state.flagged.clone();
        let root = vault.root.clone();

        self.set_flagged_from(&root, &flagged);
        self.commit(vec![Event::Status]);
    }

    // --- notification helpers (do not borrow `self.vault`) ---

    fn bound_client(&self) -> Option<(Client, String)> {
        let client = self.client.clone()?;
        let vault_id = self.vault.as_ref().and_then(|vault| vault.state.vault_id.clone())?;
        Some((client, vault_id))
    }

    fn set_status(&self, label: &str) {
        self.shared.lock().unwrap().status = label.to_string();
    }

    fn set_signed_in(&self, signed_in: bool) {
        self.shared.lock().unwrap().signed_in = signed_in;
    }

    fn set_published(&self, published: bool) {
        self.shared.lock().unwrap().published = published;
    }

    fn set_flagged_from(&self, root: &Path, relative_paths: &[String]) {
        let absolute = relative_paths.iter().map(|path| absolute(root, path)).collect();
        self.shared.lock().unwrap().flagged = absolute;
    }

    fn commit(&self, events: Vec<Event>) {
        self.shared.lock().unwrap().events.extend(events);
        self.invoker.invoke_method("pump");
    }

    fn fail(&self, message: &str) {
        self.set_status("error");
        self.commit(vec![Event::Status, Event::Failed(message.to_string())]);
    }

    /// A transport failure is just "offline" (offline-first, no alarm); a real
    /// server or merge error is shown.
    fn report(&self, error: ClientError) {
        if matches!(error, ClientError::Http(_)) {
            self.set_status("offline");
            self.commit(vec![Event::Status]);
        } else {
            self.fail(&format!("Sync failed: {error}"));
        }
    }
}

/// Runs one push+pull cycle against a bound vault, persisting state and returning
/// (changed note paths, flagged relative paths, vault root). Kept free of `self`
/// so it can hold the `&mut VaultSync` borrow without fighting the notify helpers.
fn reconcile(
    client: &Client,
    vault_id: &str,
    vault: &mut VaultSync,
    today: &str,
) -> Result<(Vec<String>, Vec<String>, PathBuf), ClientError> {
    let mut state = ClientState { cursor: vault.state.cursor, versions: vault.state.versions.clone() };

    let outcome = engine::push(client, vault_id, &vault.root, &mut vault.manifest, &mut state, today)?;
    let touched = engine::pull(client, vault_id, &vault.root, &mut vault.manifest, &mut state)?;

    vault.state.cursor = state.cursor;
    vault.state.versions = state.versions;
    for path in &outcome.flagged {
        if !vault.state.flagged.contains(path) {
            vault.state.flagged.push(path.clone());
        }
    }
    let _ = vault.state.save(&vault.root);
    let _ = vault.manifest.save(&vault.root);

    let mut notes = outcome.changed;
    notes.extend(touched);

    Ok((notes, vault.state.flagged.clone(), vault.root.clone()))
}

fn restore_content(client: &Client, vault_id: &str, path: &str, version: u64) -> Result<Vec<u8>, ClientError> {
    let history = client.history(vault_id, path)?;
    let blob = history.versions.iter().find(|entry| entry.version == version).and_then(|entry| entry.blob.clone());

    match blob {
        Some(hash) => client.get_blob(&hash),
        None => Ok(Vec::new()),
    }
}

fn token_path() -> PathBuf {
    crate::library::default_config_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("sync-token.json")
}

fn status_json(state: &str, flagged_count: usize, signed_in: bool, published: bool) -> String {
    serde_json::json!({
        "state": state,
        "flagged_count": flagged_count,
        "signed_in": signed_in,
        "published": published,
    })
    .to_string()
}

fn string(value: &serde_json::Value, key: &str) -> String {
    value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string()
}

fn absolute(root: &Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn relative(root: &Path, absolute: &str) -> String {
    Path::new(absolute)
        .strip_prefix(root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| absolute.to_string())
}

/// Today's date as `YYYY-MM-DD` (UTC), for a conflict copy's name. Computed
/// without a date crate via the civil-from-days algorithm.
fn today() -> String {
    let days =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| (d.as_secs() / 86_400) as i64).unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1)); // the epoch
        assert_eq!(civil_from_days(10_957), (2000, 1, 1)); // a well-known Unix day number
        assert_eq!(civil_from_days(59), (1970, 3, 1)); // past a non-leap February
    }
}
