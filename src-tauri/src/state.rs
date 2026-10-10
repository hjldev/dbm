use crate::model::*;
use mysql_async::{Conn, Opts, Pool};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, Notify};

pub struct Session {
    pub connection_id: String,
    pub conn: Mutex<Option<Conn>>,
    pub options: Opts,
}
pub struct Execution {
    pub connection_id: String,
    pub options: Opts,
    // Kept locked by cancellation through KILL acknowledgement. The runner must
    // acquire it before marking completion or starting another statement.
    pub current_thread: Mutex<Option<u32>>,
    pub cancelled: AtomicBool,
    pub forced_invalid: AtomicBool,
    pub force_stop: Notify,
}
impl Execution {
    pub fn new(connection_id: String, options: Opts) -> Self {
        Self {
            connection_id,
            options,
            current_thread: Mutex::new(None),
            cancelled: AtomicBool::new(false),
            forced_invalid: AtomicBool::new(false),
            force_stop: Notify::new(),
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
pub struct AppState {
    pub lifecycle: Mutex<()>,
    pub path: PathBuf,
    pub data: Mutex<PersistentData>,
    pub passwords: Mutex<HashMap<String, String>>,
    pub pools: Mutex<HashMap<String, Pool>>,
    pub sessions: Mutex<HashMap<String, Arc<Session>>>,
    pub executions: Mutex<HashMap<String, Arc<Execution>>>,
}
impl AppState {
    pub fn load(directory: PathBuf) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("settings.json");
        let data = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            PersistentData::default()
        };
        Ok(Self {
            lifecycle: Mutex::new(()),
            path,
            data: Mutex::new(data),
            passwords: Mutex::new(HashMap::new()),
            pools: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            executions: Mutex::new(HashMap::new()),
        })
    }
    pub fn persist(&self, data: &PersistentData) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::new("CONFIG", "配置目录无效"))?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(data)?)?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .map_err(|e| AppError::from(e.error))?;
        Ok(())
    }
    async fn update_data<T>(
        &self,
        update: impl FnOnce(&mut PersistentData) -> Result<T>,
    ) -> Result<T> {
        let mut data = self.data.lock().await;
        let mut updated = data.clone();
        let value = update(&mut updated)?;
        self.persist(&updated)?;
        *data = updated;
        Ok(value)
    }
    pub async fn bootstrap(&self) -> BootstrapData {
        let data = self.data.lock().await;
        BootstrapData {
            connections: data.connections.clone(),
            settings: data.settings.clone(),
            config_path: self.path.to_string_lossy().into_owned(),
        }
    }
    pub async fn save_profile(&self, input: ConnectionInput) -> Result<ConnectionProfile> {
        let password = match input.password {
            Some(password) => Some(password),
            None => {
                self.available_password(&input.profile.id, input.profile.remember_password)
                    .await?
            }
        };
        if input.profile.remember_password && password.is_none() {
            return Err(AppError::new("PASSWORD_REQUIRED", "请填写需要保存的密码。"));
        }
        self.update_data(|data| {
            if let Some(profile) = data
                .connections
                .iter_mut()
                .find(|p| p.id == input.profile.id)
            {
                *profile = input.profile.clone();
            } else {
                data.connections.push(input.profile.clone());
            }
            if input.profile.remember_password {
                if let Some(password) = &password {
                    data.passwords
                        .insert(input.profile.id.clone(), password.clone());
                }
            } else {
                data.passwords.remove(&input.profile.id);
            }
            Ok(())
        })
        .await?;
        if let Some(password) = password {
            self.passwords
                .lock()
                .await
                .insert(input.profile.id.clone(), password);
        }
        Ok(input.profile)
    }
    pub async fn delete_profile(&self, id: &str) -> Result<()> {
        self.update_data(|data| {
            data.connections.retain(|p| p.id != id);
            data.passwords.remove(id);
            Ok(())
        })
        .await?;
        self.passwords.lock().await.remove(id);
        Ok(())
    }
    // Called only after authentication succeeds, including legacy Keychain migration.
    pub async fn remember_authenticated_password(&self, id: &str, password: &str) -> Result<()> {
        let mut data = self.data.lock().await;
        if !data
            .connections
            .iter()
            .any(|p| p.id == id && p.remember_password)
            || data
                .passwords
                .get(id)
                .is_some_and(|saved| saved == password)
        {
            return Ok(());
        }
        let mut updated = data.clone();
        updated.passwords.insert(id.into(), password.into());
        self.persist(&updated)?;
        *data = updated;
        Ok(())
    }
    pub async fn profile(&self, id: &str) -> Result<ConnectionProfile> {
        self.data
            .lock()
            .await
            .connections
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| AppError::new("NOT_FOUND", "连接配置不存在"))
    }
    pub async fn password(&self, profile: &ConnectionProfile) -> Result<String> {
        self.available_password(&profile.id, true)
            .await?
            .ok_or_else(|| {
                AppError::new(
                    "PASSWORD_REQUIRED",
                    "请输入此连接的密码（未保存的密码会在断开后清除）。",
                )
            })
    }
    async fn available_password(&self, id: &str, include_legacy: bool) -> Result<Option<String>> {
        if let Some(value) = self.passwords.lock().await.get(id).cloned() {
            return Ok(Some(value));
        }
        let legacy = {
            let data = self.data.lock().await;
            if let Some(value) = data.passwords.get(id) {
                return Ok(Some(value.clone()));
            }
            data.connections
                .iter()
                .any(|p| p.id == id && p.remember_password)
        };
        if include_legacy && legacy {
            let id = id.to_owned();
            return tokio::task::spawn_blocking(move || read_optional_password(&id))
                .await
                .map_err(|_| legacy_password_error())?
                .map_err(|_| legacy_password_error());
        }
        Ok(None)
    }
    pub async fn pool(&self, id: &str) -> Result<Pool> {
        self.pools
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::new("NOT_CONNECTED", "请先连接数据库"))
    }
    pub async fn session(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::new("SESSION_LOST", "SQL 会话已关闭，请重新打开查询标签。"))
    }
    pub async fn register_execution(&self, id: &str, execution: Arc<Execution>) -> Result<()> {
        let mut executions = self.executions.lock().await;
        if executions.contains_key(id) {
            return Err(AppError::new("BUSY", "执行 ID 已被使用"));
        }
        executions.insert(id.into(), execution);
        Ok(())
    }
}
fn legacy_password_error() -> AppError {
    AppError::new(
        "PASSWORD_REQUIRED",
        "无法读取旧版保存的密码，请重新输入；记住的密码将保存到配置文件。",
    )
}
#[cfg(target_os = "macos")]
pub fn read_password(id: &str) -> Result<String> {
    read_optional_password(id)?.ok_or_else(|| {
        AppError::new(
            "PASSWORD_REQUIRED",
            "钥匙串中没有此连接的密码，请重新输入。",
        )
    })
}
#[cfg(target_os = "macos")]
pub fn read_optional_password(id: &str) -> Result<Option<String>> {
    match security_framework::passwords::get_generic_password("dev.dbm.desktop", id) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| AppError::new("KEYCHAIN", "钥匙串密码编码错误")),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(AppError::new("KEYCHAIN", error.to_string())),
    }
}
#[cfg(target_os = "macos")]
pub fn store_password(id: &str, password: &str) -> Result<()> {
    security_framework::passwords::set_generic_password("dev.dbm.desktop", id, password.as_bytes())
        .map_err(|e| AppError::new("KEYCHAIN", e.to_string()))
}
#[cfg(target_os = "macos")]
pub fn delete_password(id: &str) -> Result<()> {
    match security_framework::passwords::delete_generic_password("dev.dbm.desktop", id) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == -25300 => Ok(()),
        Err(e) => Err(AppError::new("KEYCHAIN", e.to_string())),
    }
}
#[cfg(not(target_os = "macos"))]
pub fn read_password(_: &str) -> Result<String> {
    Err(AppError::new("KEYCHAIN", "第一版的凭据存储仅支持 macOS"))
}
#[cfg(not(target_os = "macos"))]
pub fn read_optional_password(_: &str) -> Result<Option<String>> {
    Ok(None)
}
#[cfg(not(target_os = "macos"))]
pub fn store_password(_: &str, _: &str) -> Result<()> {
    Err(AppError::new("KEYCHAIN", "第一版的凭据存储仅支持 macOS"))
}
#[cfg(not(target_os = "macos"))]
pub fn delete_password(_: &str) -> Result<()> {
    Ok(())
}
