use dbm_lib::{model::*, state::AppState};

fn profile(id: &str, remember_password: bool) -> ConnectionProfile {
    ConnectionProfile {
        id: id.into(),
        name: "本地测试".into(),
        host: "127.0.0.1".into(),
        port: 3306,
        username: "root".into(),
        database: None,
        tls: false,
        ca_pem: None,
        ca_name: None,
        remember_password,
    }
}

#[tokio::test]
async fn remembered_password_survives_disconnect_and_restart_without_ipc_exposure() {
    let directory = tempfile::tempdir().unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    let profile = profile("saved", true);
    let password = "public-config-fixture-中文\n\"\\";
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some(password.into()),
        })
        .await
        .unwrap();
    // Disconnect clears only the volatile cache; the next connection uses the file.
    state.passwords.lock().await.clear();
    assert_eq!(state.password(&profile).await.unwrap(), password);
    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state.path).unwrap()).unwrap();
    assert_eq!(file["passwords"]["saved"], password);
    drop(state);
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert_eq!(restarted.password(&profile).await.unwrap(), password);
    let ipc = serde_json::to_value(restarted.bootstrap().await).unwrap();
    assert!(ipc.get("passwords").is_none());
    assert!(ipc["connections"][0].get("password").is_none());
    assert_eq!(ipc["configPath"], restarted.path.to_string_lossy().as_ref());
    assert!(!serde_json::to_string(&ipc)
        .unwrap()
        .contains("public-config-fixture"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&restarted.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn editing_preserves_password_and_explicit_empty_password_is_saved() {
    let directory = tempfile::tempdir().unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    let mut profile = profile("edit", true);
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some("original".into()),
        })
        .await
        .unwrap();
    state.passwords.lock().await.clear();
    profile.name = "重命名连接".into();
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: None,
        })
        .await
        .unwrap();
    assert_eq!(state.password(&profile).await.unwrap(), "original");
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some(String::new()),
        })
        .await
        .unwrap();
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert_eq!(restarted.password(&profile).await.unwrap(), "");
    assert_eq!(restarted.profile("edit").await.unwrap().name, "重命名连接");
}

#[tokio::test]
async fn forgetting_and_deleting_remove_only_the_corresponding_saved_password() {
    let directory = tempfile::tempdir().unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    let mut first = profile("first", true);
    let second = profile("second", true);
    for profile in [&first, &second] {
        state
            .save_profile(ConnectionInput {
                profile: profile.clone(),
                password: Some(profile.id.clone()),
            })
            .await
            .unwrap();
    }
    first.remember_password = false;
    state
        .save_profile(ConnectionInput {
            profile: first.clone(),
            password: None,
        })
        .await
        .unwrap();
    state.passwords.lock().await.clear();
    assert_eq!(
        state.password(&first).await.unwrap_err().code,
        "PASSWORD_REQUIRED"
    );
    assert_eq!(state.password(&second).await.unwrap(), "second");
    // Re-enabling remembering requires a password if it was previously forgotten.
    first.remember_password = true;
    assert_eq!(
        state
            .save_profile(ConnectionInput {
                profile: first,
                password: None
            })
            .await
            .err()
            .unwrap()
            .code,
        "PASSWORD_REQUIRED"
    );
    state.delete_profile("second").await.unwrap();
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert!(restarted.data.lock().await.passwords.is_empty());
    assert_eq!(
        restarted.profile("second").await.err().unwrap().code,
        "NOT_FOUND"
    );
}

#[tokio::test]
async fn failed_config_writes_leave_profiles_passwords_and_cache_intact() {
    let directory = tempfile::tempdir().unwrap();
    let mut state = AppState::load(directory.path().into()).unwrap();
    let mut profile = profile("atomic", true);
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some("original".into()),
        })
        .await
        .unwrap();
    let original_path = state.path.clone();
    state.path = directory.path().join("missing-parent/settings.json");
    profile.name = "不应保存".into();
    assert!(state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some("replacement".into())
        })
        .await
        .is_err());
    assert!(state
        .remember_authenticated_password("atomic", "authenticated-replacement")
        .await
        .is_err());
    assert!(state.delete_profile("atomic").await.is_err());
    assert_eq!(state.password(&profile).await.unwrap(), "original");
    assert_eq!(state.profile("atomic").await.unwrap().name, "本地测试");
    assert_eq!(
        state.data.lock().await.passwords.get("atomic").unwrap(),
        "original"
    );
    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(original_path).unwrap()).unwrap();
    assert_eq!(file["passwords"]["atomic"], "original");
}

#[tokio::test]
async fn authenticated_password_updates_are_persistent_and_respect_remembering() {
    let directory = tempfile::tempdir().unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    let profile = profile("authenticated", true);
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: Some("old".into()),
        })
        .await
        .unwrap();
    state
        .remember_authenticated_password(&profile.id, "new")
        .await
        .unwrap();
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert_eq!(restarted.password(&profile).await.unwrap(), "new");
    let temporary = crate::profile("temporary", false);
    state
        .save_profile(ConnectionInput {
            profile: temporary.clone(),
            password: Some("temporary-secret".into()),
        })
        .await
        .unwrap();
    state
        .remember_authenticated_password(&temporary.id, "new-temporary-secret")
        .await
        .unwrap();
    assert!(!state
        .data
        .lock()
        .await
        .passwords
        .contains_key(&temporary.id));
    let restarted = AppState::load(directory.path().into()).unwrap();
    assert_eq!(
        restarted.password(&temporary).await.unwrap_err().code,
        "PASSWORD_REQUIRED"
    );
}

#[tokio::test]
async fn old_configuration_loads_without_password_map_and_can_forget_legacy_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let profile = profile("legacy", true);
    let old = serde_json::json!({ "connections": [profile], "settings": Settings::default() });
    std::fs::write(
        directory.path().join("settings.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    let state = AppState::load(directory.path().into()).unwrap();
    assert!(state.data.lock().await.passwords.is_empty());
    let mut profile = state.profile("legacy").await.unwrap();
    profile.remember_password = false;
    // Forgetting old credentials must not depend on Keychain access or authorization.
    state
        .save_profile(ConnectionInput {
            profile: profile.clone(),
            password: None,
        })
        .await
        .unwrap();
    assert_eq!(
        state.password(&profile).await.unwrap_err().code,
        "PASSWORD_REQUIRED"
    );
}
