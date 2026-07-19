//! End-to-end: two devices reconciling through a real server on a random port.
//!
//! This is the M2 2a–2d exit criterion — two clients merging where they can and
//! writing a conflict copy where there is no ancestor, all with no UI. The server
//! runs on the test's tokio runtime (so its Postgres pool stays on the runtime it
//! was made on); the blocking clients run on a `spawn_blocking` thread, exactly
//! the split the real app uses.

use booklet_core::Manifest;
use booklet_sync_client::{pull, push, Client, ClientState};
use booklet_sync_proto as proto;
use booklet_sync_server::admin::AppState;
use booklet_sync_server::{app, auth, store::Store};
use sqlx::PgPool;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const TODAY: &str = "2026-07-17";
const BASE_NOTE: &str = "# Title\n\nAlpha.\n\nBeta.\n";

#[sqlx::test(migrations = "../booklet-sync-server/migrations")]
async fn two_devices_reconcile(pool: PgPool) {
    let store = Arc::new(Store::from_parts(pool, temp("blobs"), 50));
    store.create_user("alice", &auth::hash_password("pw").unwrap()).await.unwrap();

    // Serve on an ephemeral port, on this runtime.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = AppState::new(store, Instant::now());
    tokio::spawn(async move { axum::serve(listener, app::app(state)).await.unwrap() });

    // The blocking client scenario runs off the async runtime; a panicked
    // assertion inside it fails the test through the join.
    tokio::task::spawn_blocking(move || scenario(&base)).await.unwrap();
}

fn scenario(base: &str) {
    let (dir_a, dir_b) = (temp("device-a"), temp("device-b"));
    let alice = Client::login(base, &login("laptop")).unwrap();
    let alice_phone = Client::login(base, &login("phone")).unwrap();

    let mut vault_a = Sync::new();
    let mut vault_b = Sync::new();

    // --- Device A publishes a vault and pushes a note; device B pulls it. ---
    let vault = alice.publish("Personal").unwrap();
    write(&dir_a, "Note.md", BASE_NOTE);
    push(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state, TODAY).unwrap();

    pull(&alice_phone, &vault, &dir_b, &mut vault_b.manifest, &mut vault_b.state).unwrap();
    assert_eq!(read(&dir_b, "Note.md"), BASE_NOTE);

    // --- Both edit different paragraphs offline; A pushes first, B merges. ---
    write(&dir_a, "Note.md", "# Title\n\nAlpha, edited by A.\n\nBeta.\n");
    write(&dir_b, "Note.md", "# Title\n\nAlpha.\n\nBeta, edited by B.\n");

    push(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state, TODAY).unwrap();
    let merged = push(&alice_phone, &vault, &dir_b, &mut vault_b.manifest, &mut vault_b.state, TODAY).unwrap();

    // B's push hit a 409 and merged both edits cleanly (different paragraphs).
    assert!(merged.flagged.is_empty(), "a clean merge should not be flagged");
    let note_b = read(&dir_b, "Note.md");
    assert!(note_b.contains("edited by A") && note_b.contains("edited by B"), "both edits survive: {note_b:?}");

    // A pulls and converges on the same merged text.
    pull(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state).unwrap();
    let note_a = read(&dir_a, "Note.md");
    assert!(note_a.contains("edited by A") && note_a.contains("edited by B"));
    assert_eq!(note_a, note_b, "both devices converge");

    // --- No common ancestor: both create the same new filename independently. ---
    write(&dir_a, "Solo.md", "A's solo\n");
    write(&dir_b, "Solo.md", "B's solo\n");

    push(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state, TODAY).unwrap();
    let outcome = push(&alice_phone, &vault, &dir_b, &mut vault_b.manifest, &mut vault_b.state, TODAY).unwrap();

    // B could not merge (no ancestor), so it wrote a conflict copy: the server's
    // note keeps the name, B's text is preserved beside it.
    assert_eq!(outcome.conflict_copies, ["Solo (conflict 2026-07-17).md"]);
    assert_eq!(read(&dir_b, "Solo.md"), "A's solo\n");
    assert_eq!(read(&dir_b, "Solo (conflict 2026-07-17).md"), "B's solo\n");

    // And the conflict copy syncs like any other note — A pulls it.
    pull(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state).unwrap();
    assert_eq!(read(&dir_a, "Solo (conflict 2026-07-17).md"), "B's solo\n");

    // --- A subfolder (a book/section) pushes without an "is a directory" error,
    //     and the folder itself syncs to B. ---
    fs::create_dir_all(dir_a.join("Chapter")).unwrap();
    write(&dir_a.join("Chapter"), "Inside.md", "# Inside\n");
    push(&alice, &vault, &dir_a, &mut vault_a.manifest, &mut vault_a.state, TODAY).unwrap();

    pull(&alice_phone, &vault, &dir_b, &mut vault_b.manifest, &mut vault_b.state).unwrap();
    assert!(dir_b.join("Chapter").is_dir(), "the folder synced to B as a directory");
    assert_eq!(read(&dir_b.join("Chapter"), "Inside.md"), "# Inside\n");
}

/// One device's baseline manifest and sync state for a vault.
struct Sync {
    manifest: Manifest,
    state: ClientState,
}

impl Sync {
    fn new() -> Self {
        Self { manifest: Manifest::default(), state: ClientState::default() }
    }
}

fn login(device_name: &str) -> proto::TokenRequest {
    proto::TokenRequest {
        handle: "alice".into(),
        password: "pw".into(),
        device_name: device_name.into(),
        platform: "test".into(),
    }
}

fn temp(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("booklet-int-{}-{tag}-{unique}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap()
}
