use oo7::{Keyring, Secret};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tracing::warn;

fn make_attrs(uuid: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", "longlens"), ("uuid", uuid)])
}

/// Process-wide handle to the user's keyring.
///
/// oo7 talks to the Secret Service over D-Bus using its async-io backend, so the
/// futures are driven directly by the GLib main loop (the callers wrap them in
/// `glib::spawn_future_local`) — we don't spin up a tokio runtime of our own.
///
/// The `Mutex` both caches the open connection and serialises operations. It is
/// fair (FIFO), so a `store_password` awaited before a `get_password` always
/// completes first — which the "Save & Connect" flow relies on, as it stores a
/// password and then immediately reads it back via the `win.connect` action.
static KEYRING: OnceLock<Mutex<Option<Keyring>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Keyring>> {
    KEYRING.get_or_init(|| Mutex::new(None))
}

/// Lazily open (and unlock) the keyring, returning a reference to the cached
/// connection. Returns `None` if the Secret Service is unavailable.
async fn ensure(guard: &mut Option<Keyring>) -> Option<&Keyring> {
    if guard.is_none() {
        match Keyring::new().await {
            Ok(keyring) => {
                // Best-effort unlock of the default collection.
                if let Err(e) = keyring.unlock().await {
                    warn!("Could not unlock keyring: {e}");
                }
                *guard = Some(keyring);
            }
            Err(e) => {
                warn!("Could not open keyring: {e}");
                return None;
            }
        }
    }
    guard.as_ref()
}

pub async fn is_available() -> bool {
    let mut guard = cell().lock().await;
    ensure(&mut guard).await.is_some()
}

pub async fn store_password(uuid: &str, password: &SecretString) {
    let mut guard = cell().lock().await;
    let Some(keyring) = ensure(&mut guard).await else {
        return;
    };
    if let Err(e) = keyring
        .create_item(
            &format!("LongLens: {uuid}"),
            &make_attrs(uuid),
            Secret::text(password.expose_secret()),
            true,
        )
        .await
    {
        warn!("Could not store password: {e}");
    }
}

pub async fn get_password(uuid: &str) -> Option<SecretString> {
    let mut guard = cell().lock().await;
    let keyring = ensure(&mut guard).await?;
    let items = keyring.search_items(&make_attrs(uuid)).await.ok()?;
    let secret = items.first()?.secret().await.ok()?;
    String::from_utf8(secret.as_bytes().to_vec())
        .ok()
        .map(SecretString::new)
}

pub async fn delete_password(uuid: &str) {
    let mut guard = cell().lock().await;
    let Some(keyring) = ensure(&mut guard).await else {
        return;
    };
    if let Err(e) = keyring.delete(&make_attrs(uuid)).await {
        warn!("Could not delete password: {e}");
    }
}
