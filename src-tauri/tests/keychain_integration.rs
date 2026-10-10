#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires the macOS login keychain; creates and deletes its own unique fixture"]
fn keychain_roundtrip_and_missing_password() {
    use dbm_lib::state::{delete_password, read_optional_password, read_password, store_password};
    let id = format!("dbm-test-{}", uuid::Uuid::new_v4());
    let password = "public-local-keychain-fixture-中文";
    store_password(&id, password).unwrap();
    let loaded = read_password(&id);
    // Delete before asserting so a failing read never leaves a test credential.
    delete_password(&id).unwrap();
    assert_eq!(loaded.unwrap(), password);
    assert!(read_optional_password(&id).unwrap().is_none());
    assert_eq!(read_password(&id).unwrap_err().code, "PASSWORD_REQUIRED");
    delete_password(&id).unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires the macOS login keychain; migrates and deletes its own unique fixture"]
async fn legacy_password_migrates_to_configuration_and_survives_keychain_removal() {
    use dbm_lib::{
        model::{ConnectionInput, ConnectionProfile, Settings},
        state::{delete_password, store_password, AppState},
    };
    let directory = tempfile::tempdir().unwrap();
    let id = format!("dbm-migration-test-{}", uuid::Uuid::new_v4());
    let password = "public-legacy-migration-fixture-中文";
    let profile = ConnectionProfile {
        id: id.clone(),
        name: "旧版连接".into(),
        host: "127.0.0.1".into(),
        port: 3306,
        username: "root".into(),
        database: None,
        tls: false,
        ca_pem: None,
        ca_name: None,
        remember_password: true,
    };
    let old = serde_json::json!({ "connections": [profile], "settings": Settings::default() });
    std::fs::write(
        directory.path().join("settings.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    store_password(&id, password).unwrap();
    let saved = state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: None,
        })
        .await;
    // Clean up before asserting, including when the migration failed.
    delete_password(&id).unwrap();
    saved.unwrap();
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert_eq!(restarted.password(&profile).await.unwrap(), password);
}
