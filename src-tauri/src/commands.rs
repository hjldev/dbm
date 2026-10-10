use crate::{
    database, engine,
    model::*,
    script,
    state::{AppState, Execution, Session},
};
use mysql_async::{prelude::Queryable, Conn, Pool};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tauri::{ipc::Channel, AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tokio::time::timeout;

async fn pooled(state: &AppState, id: &str) -> Result<Conn> {
    timeout(Duration::from_secs(10), state.pool(id).await?.get_conn())
        .await
        .map_err(|_| AppError::new("TIMEOUT", "获取数据库连接超时"))?
        .map_err(AppError::from)
}
async fn active(state: &AppState, id: &str) -> bool {
    state.pools.lock().await.contains_key(id)
        || state
            .sessions
            .lock()
            .await
            .values()
            .any(|s| s.connection_id == id)
}
async fn input_password(state: &AppState, input: &ConnectionInput) -> Result<String> {
    if let Some(password) = &input.password {
        return Ok(password.clone());
    }
    if input.profile.id.is_empty() {
        return Ok(String::new());
    }
    state.password(&input.profile).await
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapData> {
    Ok(state.bootstrap().await)
}
#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    mut input: ConnectionInput,
) -> Result<ConnectionProfile> {
    let _lifecycle = state.lifecycle.lock().await;
    if input.profile.id.is_empty() {
        input.profile.id = uuid::Uuid::new_v4().to_string();
    }
    if active(&state, &input.profile.id).await {
        return Err(AppError::new(
            "CONNECTION_ACTIVE",
            "请先断开连接，再修改连接配置。",
        ));
    }
    // Validate configuration without accessing the database or echoing passwords.
    database::options(&input.profile, "", input.profile.database.as_deref())?;
    state.save_profile(input).await
}
#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, connection_id: String) -> Result<()> {
    let _lifecycle = state.lifecycle.lock().await;
    if active(&state, &connection_id).await {
        return Err(AppError::new(
            "CONNECTION_ACTIVE",
            "请先断开连接，再删除配置。",
        ));
    }
    state.delete_profile(&connection_id).await
}
#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, input: ConnectionInput) -> Result<String> {
    let password = input_password(&state, &input).await?;
    let opts = database::options(&input.profile, &password, input.profile.database.as_deref())?;
    let mut conn = database::new_connection(opts).await?;
    let version: String = conn
        .query_first("SELECT VERSION()")
        .await?
        .unwrap_or_default();
    conn.disconnect().await?;
    Ok(version)
}
#[tauri::command]
pub async fn connect(
    state: State<'_, AppState>,
    connection_id: String,
    password: Option<String>,
) -> Result<String> {
    let _lifecycle = state.lifecycle.lock().await;
    if state.pools.lock().await.contains_key(&connection_id) {
        let mut conn = pooled(&state, &connection_id).await?;
        return Ok(conn
            .query_first("SELECT VERSION()")
            .await?
            .unwrap_or_default());
    }
    let profile = state.profile(&connection_id).await?;
    let password = match password {
        Some(value) => value,
        None => state.password(&profile).await?,
    };
    let pool = Pool::new(database::options(
        &profile,
        &password,
        profile.database.as_deref(),
    )?);
    let tested: Result<String> = async {
        let mut conn = timeout(Duration::from_secs(10), pool.get_conn())
            .await
            .map_err(|_| AppError::new("TIMEOUT", "数据库连接超时"))??;
        Ok(conn
            .query_first("SELECT VERSION()")
            .await?
            .unwrap_or_default())
    }
    .await;
    let version = match tested {
        Ok(version) => version,
        Err(error) => {
            let _ = timeout(Duration::from_secs(2), pool.disconnect()).await;
            return Err(error);
        }
    };
    // Only replace saved credentials after the server has authenticated them.
    if profile.remember_password {
        if let Err(error) = state
            .remember_authenticated_password(&connection_id, &password)
            .await
        {
            let _ = timeout(Duration::from_secs(2), pool.disconnect()).await;
            return Err(error);
        }
    }
    state
        .passwords
        .lock()
        .await
        .insert(connection_id.clone(), password);
    state.pools.lock().await.insert(connection_id, pool);
    Ok(version)
}
#[tauri::command]
pub async fn disconnect(
    state: State<'_, AppState>,
    connection_id: String,
    rollback: bool,
) -> Result<()> {
    let _lifecycle = state.lifecycle.lock().await;
    if state
        .executions
        .lock()
        .await
        .values()
        .any(|e| e.connection_id == connection_id)
    {
        return Err(AppError::new(
            "BUSY",
            "请先取消或等待正在进行的查询 / 导出。",
        ));
    }
    let sessions: Vec<_> = state
        .sessions
        .lock()
        .await
        .iter()
        .filter(|(_, s)| s.connection_id == connection_id)
        .map(|(id, s)| (id.clone(), s.clone()))
        .collect();
    let mut guards = Vec::new();
    for (_, session) in &sessions {
        guards.push(
            session
                .conn
                .try_lock()
                .map_err(|_| AppError::new("BUSY", "该连接存在正在执行的 SQL。"))?,
        );
    }
    if !rollback {
        for guard in &mut guards {
            if engine::close_requires_rollback(guard).await {
                return Err(AppError::new(
                    "TRANSACTION_OPEN",
                    "连接存在未提交事务，请确认回滚后断开。",
                ));
            }
        }
    }
    for guard in &mut guards {
        if let Some(mut conn) = guard.take() {
            if rollback {
                let _ = timeout(Duration::from_secs(3), conn.query_drop("ROLLBACK")).await;
            }
            let _ = timeout(Duration::from_secs(2), conn.disconnect()).await;
        }
    }
    let mut stored = state.sessions.lock().await;
    for (id, _) in &sessions {
        stored.remove(id);
    }
    drop(stored);
    let pool = state.pools.lock().await.remove(&connection_id);
    if let Some(pool) = pool {
        let _ = timeout(Duration::from_secs(10), pool.disconnect()).await;
    }
    state.passwords.lock().await.remove(&connection_id);
    Ok(())
}
#[tauri::command]
pub async fn list_databases(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<String>> {
    database::databases(&mut pooled(&state, &connection_id).await?).await
}
#[tauri::command]
pub async fn list_objects(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> Result<Vec<ObjectInfo>> {
    database::objects(&mut pooled(&state, &connection_id).await?, &database).await
}
#[tauri::command]
pub async fn describe_table(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
) -> Result<TableInfo> {
    database::describe(
        &mut pooled(&state, &connection_id).await?,
        &database,
        &table,
    )
    .await
}

#[tauri::command]
pub async fn open_session(
    state: State<'_, AppState>,
    connection_id: String,
    database: Option<String>,
) -> Result<String> {
    let _lifecycle = state.lifecycle.lock().await;
    state.pool(&connection_id).await?;
    let profile = state.profile(&connection_id).await?;
    let password = state.password(&profile).await?;
    let options = database::options(
        &profile,
        &password,
        database.as_deref().or(profile.database.as_deref()),
    )?;
    let conn = database::new_connection(options.clone()).await?;
    let id = uuid::Uuid::new_v4().to_string();
    state.sessions.lock().await.insert(
        id.clone(),
        Arc::new(Session {
            connection_id,
            options,
            conn: tokio::sync::Mutex::new(Some(conn)),
        }),
    );
    Ok(id)
}
#[tauri::command]
pub async fn close_session(
    state: State<'_, AppState>,
    session_id: String,
    rollback: bool,
) -> Result<()> {
    let _lifecycle = state.lifecycle.lock().await;
    let session = match state.sessions.lock().await.get(&session_id).cloned() {
        Some(s) => s,
        None => return Ok(()),
    };
    let mut guard = session
        .conn
        .try_lock()
        .map_err(|_| AppError::new("BUSY", "请先取消或等待此标签中的查询。"))?;
    if !rollback && engine::close_requires_rollback(&mut guard).await {
        return Err(AppError::new(
            "TRANSACTION_OPEN",
            "此标签存在未提交事务，关闭将回滚。",
        ));
    }
    if let Some(mut conn) = guard.take() {
        if rollback {
            let _ = timeout(Duration::from_secs(3), conn.query_drop("ROLLBACK")).await;
        }
        let _ = timeout(Duration::from_secs(2), conn.disconnect()).await;
    }
    state.sessions.lock().await.remove(&session_id);
    Ok(())
}
#[tauri::command]
pub async fn execute_script(
    state: State<'_, AppState>,
    session_id: String,
    execution_id: String,
    source: String,
    preview_limit: usize,
    events: Channel<ExecutionEvent>,
) -> Result<ExecutionReport> {
    let session = state.session(&session_id).await?;
    let mut guard = session
        .conn
        .try_lock()
        .map_err(|_| AppError::new("BUSY", "此标签已有 SQL 正在执行。"))?;
    let conn = guard
        .as_mut()
        .ok_or_else(|| AppError::new("SESSION_LOST", "此 SQL 会话已经失效，请重新打开标签。"))?;
    let execution = Arc::new(Execution::new(
        session.connection_id.clone(),
        session.options.clone(),
    ));
    state
        .register_execution(&execution_id, execution.clone())
        .await?;
    let report = engine::execute(
        conn,
        execution_id.clone(),
        &source,
        preview_limit,
        execution,
        |event| {
            let _ = events.send(event);
        },
    )
    .await;
    state.executions.lock().await.remove(&execution_id);
    let invalid = match &report {
        Ok(report) => !report.session_valid,
        Err(error) => matches!(error.code.as_str(), "CONNECTION" | "SESSION_LOST"),
    };
    if invalid {
        if let Some(conn) = guard.take() {
            let _ = timeout(Duration::from_secs(2), conn.disconnect()).await;
        }
    }
    report
}
#[tauri::command]
pub async fn cancel_execution(state: State<'_, AppState>, execution_id: String) -> Result<()> {
    let execution = state.executions.lock().await.get(&execution_id).cloned();
    if let Some(execution) = execution {
        engine::cancel(execution).await?;
    }
    Ok(())
}
#[tauri::command]
pub async fn statement_at_cursor(
    state: State<'_, AppState>,
    source: String,
    cursor: usize,
    session_id: Option<String>,
) -> Result<String> {
    let mode = if let Some(id) = session_id {
        let session = state.session(&id).await?;
        let mut guard = session
            .conn
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "SQL 会话正在执行"))?;
        let conn = guard
            .as_mut()
            .ok_or_else(|| AppError::new("SESSION_LOST", "SQL 会话已失效"))?;
        engine::snapshot(conn).await?.mode
    } else {
        script::SqlMode::default()
    };
    script::at_cursor(&source, cursor, mode)
}
#[tauri::command]
pub async fn read_page(state: State<'_, AppState>, request: PageRequest) -> Result<PageResult> {
    database::page(&mut pooled(&state, &request.connection_id).await?, &request).await
}
#[tauri::command]
pub async fn apply_changes(state: State<'_, AppState>, request: ChangeRequest) -> Result<u64> {
    database::apply(&mut pooled(&state, &request.connection_id).await?, &request).await
}

async fn save_dialog(
    app: AppHandle,
    name: String,
    extension: &'static str,
) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || {
        let name = std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export");
        app.dialog()
            .file()
            .add_filter(extension.to_uppercase(), &[extension])
            .set_file_name(name)
            .blocking_save_file()
            .map(|file| {
                file.into_path()
                    .map_err(|e| AppError::new("IO", e.to_string()))
            })
            .transpose()
    })
    .await
    .map_err(|e| AppError::new("IO", e.to_string()))?
}
async fn open_dialog(
    app: AppHandle,
    extensions: Vec<&'static str>,
    label: &'static str,
) -> Result<Option<PathBuf>> {
    tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(label, &extensions)
            .blocking_pick_file()
            .map(|file| {
                file.into_path()
                    .map_err(|e| AppError::new("IO", e.to_string()))
            })
            .transpose()
    })
    .await
    .map_err(|e| AppError::new("IO", e.to_string()))?
}
async fn read_selected(path: PathBuf, max: u64) -> Result<FileContent> {
    if tokio::fs::metadata(&path).await?.len() > max {
        return Err(AppError::new(
            "FILE_TOO_LARGE",
            "文件超过此操作支持的大小。",
        ));
    }
    let content = tokio::fs::read_to_string(&path).await?;
    Ok(FileContent {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        content,
    })
}
#[tauri::command]
pub async fn pick_ca(app: AppHandle) -> Result<Option<FileContent>> {
    match open_dialog(app, vec!["pem", "crt", "cer"], "CA 证书").await? {
        Some(path) => Ok(Some(read_selected(path, 1_048_576).await?)),
        None => Ok(None),
    }
}
#[tauri::command]
pub async fn open_sql_file(app: AppHandle) -> Result<Option<FileContent>> {
    match open_dialog(app, vec!["sql", "txt"], "SQL 文件").await? {
        Some(path) => Ok(Some(read_selected(path, 10_485_760).await?)),
        None => Ok(None),
    }
}
#[tauri::command]
pub async fn save_sql_file(app: AppHandle, source: String, name: String) -> Result<Option<String>> {
    let Some(path) = save_dialog(app, name, "sql").await? else {
        return Ok(None);
    };
    let display = path.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || atomic_write(&path, source.as_bytes()))
        .await
        .map_err(|e| AppError::new("IO", e.to_string()))??;
    Ok(Some(display))
}
fn atomic_write(path: &std::path::Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| AppError::new("IO", "无效文件路径"))?,
    )?;
    file.write_all(content)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| AppError::from(e.error))?;
    Ok(())
}
#[tauri::command]
pub async fn export_preview(app: AppHandle, result: ResultSet) -> Result<Option<String>> {
    let Some(path) = save_dialog(app, "query-result.csv".into(), "csv").await? else {
        return Ok(None);
    };
    let display = path.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || {
        let mut file = tempfile::NamedTempFile::new_in(
            path.parent()
                .ok_or_else(|| AppError::new("IO", "无效导出路径"))?,
        )?;
        {
            let mut data = csv::Writer::from_writer(file.as_file_mut());
            data.write_record(result.columns.iter().map(|c| &c.name))?;
            for row in result.rows {
                data.write_record(row.iter().map(CellValue::display))?;
            }
            data.flush()?;
        }
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| AppError::from(e.error))?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError::new("IO", e.to_string()))??;
    Ok(Some(display))
}
#[tauri::command]
pub async fn export_table(
    app: AppHandle,
    state: State<'_, AppState>,
    request: PageRequest,
    execution_id: String,
    events: Channel<ExportProgress>,
) -> Result<Option<String>> {
    let Some(path) = save_dialog(app, format!("{}.csv", request.table), "csv").await? else {
        return Ok(None);
    };
    let lifecycle = state.lifecycle.lock().await;
    state.pool(&request.connection_id).await?;
    let profile = state.profile(&request.connection_id).await?;
    let password = state.password(&profile).await?;
    // Export owns a dedicated connection so a forced cancel cannot poison a pool.
    let options = database::options(&profile, &password, Some(&request.database))?;
    let mut conn = database::new_connection(options.clone()).await?;
    let info = database::describe(&mut conn, &request.database, &request.table).await?;
    let (sql, params) = database::table_query(&info, &request, false)?;
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| AppError::new("IO", "无效导出路径"))?,
    )?;
    let execution = Arc::new(Execution::new(request.connection_id.clone(), options));
    state
        .register_execution(&execution_id, execution.clone())
        .await?;
    let cancelled = {
        let mut current = execution.current_thread.lock().await;
        if execution.is_cancelled() {
            true
        } else {
            *current = Some(conn.id());
            false
        }
    };
    drop(lifecycle);
    if cancelled {
        state.executions.lock().await.remove(&execution_id);
        let _ = timeout(Duration::from_secs(2), conn.disconnect()).await;
        return Err(AppError::new("CANCELLED", "导出已取消。"));
    }
    let display = path.to_string_lossy().into_owned();
    let exported = tokio::select! {
        result = stream_csv(&mut conn, sql, params, file.as_file_mut(), &execution_id, &events, &execution) => result,
        _ = execution.force_stop.notified() => Err(AppError::new("CANCELLED", "导出已取消。")),
    };
    *execution.current_thread.lock().await = None;
    state.executions.lock().await.remove(&execution_id);
    let _ = timeout(Duration::from_secs(2), conn.disconnect()).await;
    if execution.is_cancelled() {
        return Err(AppError::new(
            "CANCELLED",
            "导出已取消，原目标文件保持不变。",
        ));
    }
    exported?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| AppError::from(e.error))?;
    Ok(Some(display))
}
async fn stream_csv(
    conn: &mut Conn,
    sql: String,
    params: Vec<mysql_async::Value>,
    file: &mut std::fs::File,
    id: &str,
    events: &Channel<ExportProgress>,
    execution: &Execution,
) -> Result<()> {
    let mut result = conn.exec_iter(sql, params).await?;
    let columns = result.columns_ref().to_vec();
    let mut writer = csv::Writer::from_writer(file);
    writer.write_record(columns.iter().map(|c| c.name_str().into_owned()))?;
    let mut count = 0;
    let _ = events.send(ExportProgress {
        execution_id: id.into(),
        rows: count,
    });
    while let Some(row) = result.next().await? {
        // Keep consuming until KILL reaches EOF; do not return a partial connection to a pool.
        if !execution.is_cancelled() {
            writer.write_record(
                database::row_values(row, &columns)?
                    .iter()
                    .map(CellValue::display),
            )?;
            count += 1;
        }
        if count % 500 == 0 {
            writer.flush()?;
            let _ = events.send(ExportProgress {
                execution_id: id.into(),
                rows: count,
            });
            tokio::task::yield_now().await;
        }
    }
    writer.flush()?;
    let _ = events.send(ExportProgress {
        execution_id: id.into(),
        rows: count,
    });
    Ok(())
}
#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<()> {
    if !["system", "light", "dark"].contains(&settings.theme.as_str())
        || settings.preview_limit == 0
        || settings.preview_limit > 100_000
        || !(180..=480).contains(&settings.sidebar_width)
    {
        return Err(AppError::new("VALIDATION", "设置值无效。"));
    }
    let mut data = state.data.lock().await;
    let previous = data.settings.clone();
    data.settings = settings;
    if let Err(error) = state.persist(&data) {
        data.settings = previous;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_replace_and_csv_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.csv");
        atomic_write(&path, b"old").unwrap();
        let mut csv = csv::Writer::from_writer(Vec::new());
        csv.write_record(["中文", "a,b", "quote\"", "line\nnext", "\\N"])
            .unwrap();
        let bytes = csv.into_inner().unwrap();
        atomic_write(&path, &bytes).unwrap();
        let rows = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)
            .unwrap()
            .records()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&rows[0][1], "a,b");
        assert_eq!(&rows[0][3], "line\nnext");
        assert_eq!(&rows[0][4], "\\N");
    }
}
