use secret_service::{EncryptionType, SecretService};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;

fn make_attrs(uuid: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", "longlens"), ("uuid", uuid)])
}

pub fn is_available() -> bool {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            SecretService::connect(EncryptionType::Dh).await.is_ok()
        })
}

pub fn store_password(uuid: &str, password: &SecretString) {
    let uuid = uuid.to_owned();
    let password_bytes = password.expose_secret().as_bytes().to_owned();
    let _ = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let ss = SecretService::connect(EncryptionType::Dh).await?;
            let collection = ss.get_default_collection().await?;
            collection.ensure_unlocked().await?;
            collection
                .create_item(
                    &format!("LongLens: {}", uuid),
                    make_attrs(&uuid),
                    &password_bytes,
                    true,
                    "text/plain",
                )
                .await?;
            Ok::<(), secret_service::Error>(())
        });
}

pub fn get_password(uuid: &str) -> Option<SecretString> {
    let uuid = uuid.to_owned();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let ss = SecretService::connect(EncryptionType::Dh).await.ok()?;
            let collection = ss.get_default_collection().await.ok()?;
            collection.ensure_unlocked().await.ok()?;
            let items = collection.search_items(make_attrs(&uuid)).await.ok()?;
            let secret = items.first()?.get_secret().await.ok()?;
            String::from_utf8(secret).ok().map(SecretString::new)
        })
}

pub fn delete_password(uuid: &str) {
    let uuid = uuid.to_owned();
    let _ = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let ss = SecretService::connect(EncryptionType::Dh).await?;
            let collection = ss.get_default_collection().await?;
            collection.ensure_unlocked().await?;
            let items = collection.search_items(make_attrs(&uuid)).await?;
            for item in items {
                item.delete().await?;
            }
            Ok::<(), secret_service::Error>(())
        });
}
