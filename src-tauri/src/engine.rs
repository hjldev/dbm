use crate::{
    database,
    model::*,
    script::{self, SqlMode},
    state::Execution,
};
use mysql_async::{consts::StatusFlags, prelude::Queryable, Conn};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::time::timeout;

pub struct SessionSnapshot {
    pub database: Option<String>,
    pub mode: SqlMode,
    pub in_transaction: bool,
}
pub async fn snapshot(conn: &mut Conn) -> Result<SessionSnapshot> {
    let (database, mode): (Option<String>, String) = conn
        .query_first("SELECT DATABASE(), @@SESSION.sql_mode")
        .await?
        .ok_or_else(|| AppError::new("CONNECTION", "无法读取数据库会话状态"))?;
    let in_transaction = conn.last_ok_packet().is_some_and(|p| {
        p.status_flags()
            .contains(StatusFlags::SERVER_STATUS_IN_TRANS)
    });
    Ok(SessionSnapshot {
        database,
        mode: SqlMode::parse(&mode),
        in_transaction,
    })
}

/// An unreachable session cannot be recovered or committed. Drop its transport
/// instead of trapping the user in a tab that can no longer be closed.
pub async fn close_requires_rollback(conn: &mut Option<Conn>) -> bool {
    let Some(active) = conn.as_mut() else {
        return false;
    };
    match timeout(Duration::from_secs(5), snapshot(active)).await {
        Ok(Ok(state)) => state.in_transaction,
        _ => {
            conn.take();
            false
        }
    }
}

async fn consume(
    conn: &mut Conn,
    sql: &str,
    limit: usize,
) -> Result<(Vec<ResultSet>, String, Option<String>, u16)> {
    let mut result = conn.query_iter(sql).await?;
    let mut sets = Vec::new();
    loop {
        let columns = result.columns_ref().to_vec();
        let mut rows = Vec::new();
        let mut total_rows = 0;
        while let Some(row) = result.next().await? {
            total_rows += 1;
            if rows.len() < limit {
                rows.push(database::row_values(row, &columns)?);
            }
        }
        if !columns.is_empty() {
            sets.push(ResultSet {
                columns: columns.iter().map(database::column_meta).collect(),
                truncated: total_rows > rows.len() as u64,
                total_rows,
                rows,
            });
        }
        if result.is_empty() {
            break;
        }
    }
    let summary = (
        sets,
        result.affected_rows().to_string(),
        result.last_insert_id().map(|v| v.to_string()),
        result.warnings(),
    );
    // CALL may defer a later server error even when is_empty() is already true.
    result.drop_result().await?;
    Ok(summary)
}

/// Runs SQL on a dedicated session connection. The owner must close the connection
/// if session_valid is false; interrupted protocol reads must never be reused.
pub async fn execute<F: FnMut(ExecutionEvent)>(
    conn: &mut Conn,
    execution_id: String,
    source: &str,
    preview_limit: usize,
    execution: Arc<Execution>,
    mut on_event: F,
) -> Result<ExecutionReport> {
    if preview_limit == 0 || preview_limit > 100_000 {
        return Err(AppError::new(
            "VALIDATION",
            "结果预览行数需在 1 到 100000 之间。",
        ));
    }
    let initial = snapshot(conn).await?;
    script::preflight(source, initial.mode)?;
    if script::next_statement(source, 0, initial.mode)?.is_none() {
        return Err(AppError::new("EMPTY_SQL", "没有可执行的 SQL。"));
    }
    let mut state = initial;
    let mut report = ExecutionReport {
        execution_id,
        completed: 0,
        cancelled: false,
        session_valid: true,
        in_transaction: state.in_transaction,
        database: state.database.clone(),
        error: None,
    };
    let mut offset = 0;
    let mut index = 0;
    loop {
        if execution.is_cancelled() {
            report.cancelled = true;
            break;
        }
        let statement = match script::next_statement(source, offset, state.mode) {
            Ok(Some(s)) => s,
            Ok(None) => break,
            Err(e) => {
                report.error = Some(e);
                break;
            }
        };
        // Serialize the boundary with cancellation so a delayed KILL cannot target
        // a subsequent statement (including an execution on the same session).
        {
            let mut current = execution.current_thread.lock().await;
            if execution.is_cancelled() {
                report.cancelled = true;
                break;
            }
            *current = Some(conn.id());
        }
        on_event(ExecutionEvent::Progress {
            index,
            line: statement.line,
            sql: statement.sql.clone(),
        });
        let start = Instant::now();
        let mut consumed = if execution.is_cancelled() {
            Err(AppError::new("CANCELLED", "执行已取消。"))
        } else {
            tokio::select! {
                value = consume(conn, &statement.sql, preview_limit) => value,
                _ = execution.force_stop.notified() => Err(AppError::new("SESSION_LOST", "取消未能及时完成，已关闭该 SQL 会话；不会自动重放任何语句。")),
            }
        };
        *execution.current_thread.lock().await = None;
        if execution.forced_invalid.load(Ordering::SeqCst) {
            consumed = Err(AppError::new(
                "SESSION_LOST",
                "服务器未能确认取消，当前会话已失效；请核实数据库状态后重新连接。",
            ));
        }
        let mut outcome = StatementOutcome {
            index,
            sql: statement.sql,
            line: statement.line,
            elapsed_ms: start.elapsed().as_millis() as u64,
            affected_rows: "0".into(),
            last_insert_id: None,
            warnings: 0,
            results: Vec::new(),
            error: None,
        };
        match consumed {
            Ok((sets, affected, insert_id, warnings)) => {
                outcome.results = sets;
                outcome.affected_rows = affected;
                outcome.last_insert_id = insert_id;
                outcome.warnings = warnings;
                report.completed += 1;
            }
            Err(mut e) => {
                e.statement_index = Some(index);
                e.line = Some(statement.line);
                report.cancelled = execution.is_cancelled();
                if e.code == "SESSION_LOST" || e.code == "CONNECTION" {
                    report.session_valid = false;
                }
                outcome.error = Some(e.clone());
                report.error = Some(e);
            }
        }
        on_event(ExecutionEvent::Statement { outcome });
        if report.session_valid {
            match timeout(Duration::from_secs(5), snapshot(conn)).await {
                Ok(Ok(next)) => {
                    state = next;
                    report.database = state.database.clone();
                    report.in_transaction = state.in_transaction;
                }
                _ => {
                    report.session_valid = false;
                    report.error.get_or_insert_with(|| {
                        AppError::new("SESSION_LOST", "数据库会话状态无法恢复，请重新连接。")
                    });
                }
            }
        }
        if report.error.is_some() || execution.is_cancelled() {
            report.cancelled |= execution.is_cancelled();
            break;
        }
        offset = statement.next;
        index += 1;
    }
    Ok(report)
}

pub async fn cancel(execution: Arc<Execution>) -> Result<()> {
    execution.cancelled.store(true, Ordering::SeqCst);
    let current = execution.current_thread.lock().await;
    if let Some(thread) = *current {
        let killed = timeout(Duration::from_secs(5), async {
            let mut control = database::new_connection(execution.options.clone()).await?;
            let result = control
                .query_drop(format!("KILL QUERY {thread}"))
                .await
                .map_err(AppError::from);
            let _ = timeout(Duration::from_secs(1), control.disconnect()).await;
            result
        })
        .await;
        match killed {
            Ok(Ok(())) => {
                let execution = execution.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if execution.current_thread.lock().await.is_some() {
                        execution.forced_invalid.store(true, Ordering::SeqCst);
                        execution.force_stop.notify_one();
                    }
                });
            }
            _ => {
                execution.forced_invalid.store(true, Ordering::SeqCst);
                execution.force_stop.notify_one();
                return Err(AppError::new(
                    "CANCEL_FAILED",
                    "服务器未确认取消，正在关闭该会话。",
                ));
            }
        }
    }
    Ok(())
}
