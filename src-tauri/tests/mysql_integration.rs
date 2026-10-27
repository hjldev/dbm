//! Opt-in tests against disposable Docker MySQL 8.0 and 8.4, never a user database.
//! Run: DBM_TEST_PORTS=13306,13307 cargo test --test mysql_integration -- --ignored
use dbm_lib::{database, engine, model::*, state::Execution};
use mysql_async::{prelude::Queryable, Conn, Opts, OptsBuilder};
use std::{sync::Arc, time::Duration};

fn ports() -> Vec<u16> {
    std::env::var("DBM_TEST_PORTS")
        .unwrap_or_else(|_| "13306,13307".into())
        .split(',')
        .map(|p| {
            p.trim()
                .parse()
                .expect("DBM_TEST_PORTS must contain port numbers")
        })
        .collect()
}
fn profile(port: u16) -> ConnectionProfile {
    ConnectionProfile {
        id: format!("integration-{port}"),
        name: "Disposable integration database".into(),
        host: "127.0.0.1".into(),
        port,
        username: "dbm".into(),
        database: Some("dbm_test".into()),
        tls: false,
        ca_pem: None,
        ca_name: None,
        remember_password: false,
    }
}
fn password() -> String {
    std::env::var("DBM_TEST_PASSWORD").unwrap_or_else(|_| "dbm_local_test_only".into())
}
fn opts(port: u16) -> Opts {
    database::options(&profile(port), &password(), Some("dbm_test")).unwrap()
}
fn request(table: &str) -> PageRequest {
    PageRequest {
        connection_id: "integration".into(),
        database: "dbm_test".into(),
        table: table.into(),
        page: 0,
        page_size: 200,
        sort_column: None,
        sort_desc: false,
        filters: vec![],
    }
}
fn change(table: &str, changes: Vec<RowChange>) -> ChangeRequest {
    ChangeRequest {
        connection_id: "integration".into(),
        database: "dbm_test".into(),
        table: table.into(),
        changes,
    }
}
fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}
fn integer(value: i64) -> CellValue {
    CellValue::Integer(value.to_string())
}
fn insert(id: i64, label: &str) -> RowChange {
    RowChange {
        kind: "insert".into(),
        original: None,
        values: vec![Some(integer(id)), Some(text(label))],
    }
}
async fn run(
    conn: &mut Conn,
    options: &Opts,
    source: &str,
    limit: usize,
) -> (ExecutionReport, Vec<StatementOutcome>) {
    let execution = Arc::new(Execution::new("integration".into(), options.clone()));
    let mut outcomes = Vec::new();
    let report = engine::execute(
        conn,
        uuid::Uuid::new_v4().to_string(),
        source,
        limit,
        execution,
        |event| {
            if let ExecutionEvent::Statement { outcome } = event {
                outcomes.push(*outcome);
            }
        },
    )
    .await
    .unwrap();
    (report, outcomes)
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn metadata_pagination_types_and_transactional_edits() {
    for port in ports() {
        let mut conn = database::new_connection(opts(port)).await.unwrap();
        assert!(database::databases(&mut conn)
            .await
            .unwrap()
            .contains(&"dbm_test".into()));
        let objects = database::objects(&mut conn, "dbm_test").await.unwrap();
        assert!(objects
            .iter()
            .any(|o| o.name == "value_summary" && o.kind == "view"));
        let composite = database::describe(&mut conn, "dbm_test", "composite_pk")
            .await
            .unwrap();
        assert!(composite.editable);
        assert_eq!(composite.key_columns, vec!["tenant_id", "item_id"]);
        let unique = database::describe(&mut conn, "dbm_test", "unique_key")
            .await
            .unwrap();
        assert!(unique.editable);
        assert_eq!(unique.key_columns, vec!["code"]);
        for table in [
            "nullable_unique",
            "no_key",
            "non_transactional",
            "value_summary",
        ] {
            assert!(
                !database::describe(&mut conn, "dbm_test", table)
                    .await
                    .unwrap()
                    .editable,
                "{table} must be read only"
            );
        }
        let child = database::describe(&mut conn, "dbm_test", "child_rows")
            .await
            .unwrap();
        assert_eq!(child.foreign_keys[0].referenced_table, "parent_rows");
        assert!(child.create_sql.contains("FOREIGN KEY"));

        let mut page = request("large_rows");
        let first = database::page(&mut conn, &page).await.unwrap();
        assert_eq!(first.result.rows.len(), 200);
        assert!(first.has_next);
        assert_eq!(first.result.rows[0][0], integer(1));
        page.page = 1;
        assert_eq!(
            database::page(&mut conn, &page).await.unwrap().result.rows[0][0],
            integer(201)
        );
        page.page = 0;
        page.sort_column = Some("id".into());
        page.sort_desc = true;
        page.filters.push(Filter {
            column: "group_id".into(),
            operator: "eq".into(),
            value: Some(integer(3)),
        });
        let filtered = database::page(&mut conn, &page).await.unwrap();
        assert_eq!(filtered.result.rows[0][0], integer(9994));
        assert!(filtered.result.rows.iter().all(|row| row[2] == integer(3)));

        let values = database::page(&mut conn, &request("value_types"))
            .await
            .unwrap();
        let row = &values.result.rows[0];
        assert_eq!(row[1], text("中文😀"));
        assert!(row[2].display().contains("你好😀"));
        assert_eq!(row[3], CellValue::Binary("AP8QQQ==".into()));
        assert_eq!(row[4], CellValue::Integer("18446744073709551615".into()));
        assert_eq!(
            row[5],
            CellValue::Decimal("123456789012345678.123456789012".into())
        );
        assert_eq!(row[6], CellValue::Null);
        assert_eq!(row[7], integer(2));
        assert_eq!(row[8], CellValue::Date("2026-10-10 12:34:56.123456".into()));

        // A fresh, ordinary InnoDB table isolates mutations from the immutable fixture.
        let table = format!("edit_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!("CREATE TABLE `{table}` (id INT PRIMARY KEY, label VARCHAR(100) NOT NULL) ENGINE=InnoDB")).await.unwrap();
        assert_eq!(
            database::apply(
                &mut conn,
                &change(&table, vec![insert(1, "before"), insert(2, "literal %_!")])
            )
            .await
            .unwrap(),
            2
        );
        let original = database::page(&mut conn, &request(&table))
            .await
            .unwrap()
            .result
            .rows[0]
            .clone();
        let update = RowChange {
            kind: "update".into(),
            original: Some(original.clone()),
            values: vec![None, Some(text("after"))],
        };
        assert_eq!(
            database::apply(&mut conn, &change(&table, vec![update]))
                .await
                .unwrap(),
            1
        );
        let stale = RowChange {
            kind: "update".into(),
            original: Some(original),
            values: vec![None, Some(text("stale"))],
        };
        let error = database::apply(
            &mut conn,
            &change(&table, vec![insert(3, "must roll back"), stale]),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "EDIT_CONFLICT");
        assert_eq!(
            conn.query_first::<u64, _>(format!("SELECT COUNT(*) FROM `{table}`"))
                .await
                .unwrap(),
            Some(2)
        );
        assert!(database::apply(
            &mut conn,
            &change(
                &table,
                vec![insert(3, "must roll back"), insert(1, "duplicate")]
            )
        )
        .await
        .is_err());
        assert_eq!(
            conn.query_first::<u64, _>(format!("SELECT COUNT(*) FROM `{table}` WHERE id=3"))
                .await
                .unwrap(),
            Some(0)
        );

        let mut literal_filter = request(&table);
        literal_filter.filters.push(Filter {
            column: "label".into(),
            operator: "contains".into(),
            value: Some(text("%_!")),
        });
        assert_eq!(
            database::page(&mut conn, &literal_filter)
                .await
                .unwrap()
                .result
                .rows
                .len(),
            1
        );
        literal_filter.filters[0].value = Some(text("' OR 1=1 --"));
        assert!(database::page(&mut conn, &literal_filter)
            .await
            .unwrap()
            .result
            .rows
            .is_empty());
        let original = database::page(&mut conn, &request(&table))
            .await
            .unwrap()
            .result
            .rows[0]
            .clone();
        assert_eq!(
            database::apply(
                &mut conn,
                &change(
                    &table,
                    vec![RowChange {
                        kind: "delete".into(),
                        original: Some(original),
                        values: vec![None, None]
                    }]
                )
            )
            .await
            .unwrap(),
            1
        );
        let readonly = database::apply(
            &mut conn,
            &change(
                "no_key",
                vec![RowChange {
                    kind: "insert".into(),
                    original: None,
                    values: vec![Some(text("rejected"))],
                }],
            ),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(readonly.code, "READ_ONLY");

        // Generated columns are preserved when omitted, and the full value row
        // can make a round trip without reducing integer/decimal precision.
        let types_table = format!("types_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!("CREATE TABLE `{types_table}` LIKE value_types"))
            .await
            .unwrap();
        let mut insert_values: Vec<_> = row.iter().cloned().map(Some).collect();
        insert_values[7] = None;
        database::apply(
            &mut conn,
            &change(
                &types_table,
                vec![RowChange {
                    kind: "insert".into(),
                    original: None,
                    values: insert_values,
                }],
            ),
        )
        .await
        .unwrap();
        let inserted = database::page(&mut conn, &request(&types_table))
            .await
            .unwrap()
            .result
            .rows[0]
            .clone();
        assert_eq!(inserted, *row);
        let mut generated_update = vec![None; inserted.len()];
        generated_update[7] = Some(integer(99));
        assert_eq!(
            database::apply(
                &mut conn,
                &change(
                    &types_table,
                    vec![RowChange {
                        kind: "update".into(),
                        original: Some(inserted),
                        values: generated_update
                    }]
                )
            )
            .await
            .err()
            .unwrap()
            .code,
            "READ_ONLY_COLUMN"
        );
        let composite_table = format!("composite_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!(
            "CREATE TABLE `{composite_table}` LIKE composite_pk"
        ))
        .await
        .unwrap();
        conn.query_drop(format!(
            "INSERT INTO `{composite_table}` SELECT * FROM composite_pk"
        ))
        .await
        .unwrap();
        let original = database::page(&mut conn, &request(&composite_table))
            .await
            .unwrap()
            .result
            .rows[0]
            .clone();
        assert_eq!(
            database::apply(
                &mut conn,
                &change(
                    &composite_table,
                    vec![RowChange {
                        kind: "update".into(),
                        original: Some(original),
                        values: vec![None, None, Some(text("composite updated"))],
                    }]
                )
            )
            .await
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_first::<String, _>(format!(
                "SELECT label FROM `{composite_table}` WHERE tenant_id=2 AND item_id=1"
            ))
            .await
            .unwrap(),
            Some("other tenant".into())
        );

        let unique_table = format!("unique_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!("CREATE TABLE `{unique_table}` LIKE unique_key"))
            .await
            .unwrap();
        conn.query_drop(format!(
            "INSERT INTO `{unique_table}` SELECT * FROM unique_key"
        ))
        .await
        .unwrap();
        let original = database::page(&mut conn, &request(&unique_table))
            .await
            .unwrap()
            .result
            .rows[0]
            .clone();
        assert_eq!(
            database::apply(
                &mut conn,
                &change(
                    &unique_table,
                    vec![RowChange {
                        kind: "update".into(),
                        original: Some(original),
                        values: vec![None, Some(text("unique updated"))],
                    }]
                )
            )
            .await
            .unwrap(),
            1
        );

        let child_table = format!("child_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!("CREATE TABLE `{child_table}` (id INT PRIMARY KEY, parent_id INT NOT NULL, label VARCHAR(100), FOREIGN KEY (parent_id) REFERENCES parent_rows(id)) ENGINE=InnoDB")).await.unwrap();
        let rows = vec![
            RowChange {
                kind: "insert".into(),
                original: None,
                values: vec![
                    Some(integer(2)),
                    Some(integer(1)),
                    Some(text("valid but rolled back")),
                ],
            },
            RowChange {
                kind: "insert".into(),
                original: None,
                values: vec![
                    Some(integer(3)),
                    Some(integer(999)),
                    Some(text("invalid FK")),
                ],
            },
        ];
        assert!(database::apply(&mut conn, &change(&child_table, rows))
            .await
            .is_err());
        assert_eq!(
            conn.query_first::<u64, _>(format!("SELECT COUNT(*) FROM `{child_table}`"))
                .await
                .unwrap(),
            Some(0)
        );
        conn.query_drop(format!("DROP TABLE `{table}`, `{types_table}`, `{composite_table}`, `{unique_table}`, `{child_table}`"))
            .await
            .unwrap();
        conn.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn scripts_sessions_modes_errors_preview_and_cancellation() {
    for port in ports() {
        let options = opts(port);
        let mut conn = database::new_connection(options.clone()).await.unwrap();
        let (report, outcomes) = run(&mut conn, &options, "USE dbm_test; SET @dbm_value=41; SELECT @dbm_value+1 AS answer, '中文😀;ok' AS greeting;", 1000).await;
        assert_eq!(report.completed, 3);
        assert!(report.error.is_none());
        assert_eq!(report.database.as_deref(), Some("dbm_test"));
        assert_eq!(outcomes[2].results[0].rows[0][0], integer(42));
        assert_eq!(outcomes[2].results[0].rows[0][1], text("中文😀;ok"));
        let (report, _) = run(&mut conn, &options, "SET sql_mode='NO_BACKSLASH_ESCAPES,ANSI_QUOTES'; SELECT 'a\\'; SELECT 2 AS \"quoted;name\";", 1000).await;
        assert_eq!(report.completed, 3);
        assert!(report.error.is_none());
        conn.query_drop("SET sql_mode=DEFAULT").await.unwrap();
        let (report, _) = run(&mut conn, &options, "SET sql_mode=CONCAT('ANSI_QUOTES', ',NO_BACKSLASH_ESCAPES'); SELECT 'a\\'; SELECT 2 AS \"quoted;name\";", 1000).await;
        assert_eq!(report.completed, 3);
        assert!(report.error.is_none());
        conn.query_drop("SET sql_mode=DEFAULT").await.unwrap();
        let (report, _) = run(&mut conn, &options, "/* change the lexical mode */ SET sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'a\\'; SELECT 2;", 1000).await;
        assert_eq!(report.completed, 3);
        assert!(report.error.is_none());
        conn.query_drop("SET sql_mode=DEFAULT").await.unwrap();

        let (report, outcomes) = run(
            &mut conn,
            &options,
            "SELECT * FROM large_rows; SELECT 7 AS next;",
            5,
        )
        .await;
        assert_eq!(report.completed, 2);
        assert_eq!(outcomes[0].results[0].rows.len(), 5);
        assert_eq!(outcomes[0].results[0].total_rows, 10000);
        assert!(outcomes[0].results[0].truncated);
        assert_eq!(outcomes[1].results[0].rows[0][0], integer(7));
        let (report, outcomes) = run(
            &mut conn,
            &options,
            "BEGIN;\nSELECT * FROM nonexistent_dbm_table;\nSET @must_not_run=1;",
            10,
        )
        .await;
        assert_eq!(report.completed, 1);
        assert!(report.session_valid);
        assert!(report.in_transaction);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(report.error.as_ref().unwrap().line, Some(2));
        assert_eq!(report.error.as_ref().unwrap().statement_index, Some(1));
        assert!(conn
            .query_first::<Option<String>, _>("SELECT @must_not_run")
            .await
            .unwrap()
            .unwrap()
            .is_none());
        run(&mut conn, &options, "ROLLBACK;", 10).await;
        assert!(!engine::snapshot(&mut conn).await.unwrap().in_transaction);

        let execution = Arc::new(Execution::new("integration".into(), options.clone()));
        let err = engine::execute(
            &mut conn,
            "preflight".into(),
            "SET @must_not_run=2; DELIMITER $$",
            10,
            execution,
            |_| {},
        )
        .await
        .err()
        .unwrap();
        assert_eq!(err.code, "UNSUPPORTED_DELIMITER");
        assert!(conn
            .query_first::<Option<String>, _>("SELECT @must_not_run")
            .await
            .unwrap()
            .unwrap()
            .is_none());

        let execution = Arc::new(Execution::new("integration".into(), options.clone()));
        let runner_execution = execution.clone();
        let handle = tokio::spawn(async move {
            let report = engine::execute(
                &mut conn,
                "cancel-sleep".into(),
                "SELECT SLEEP(30); SET @cancelled_tail=1;",
                10,
                runner_execution,
                |_| {},
            )
            .await
            .unwrap();
            (conn, report)
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if execution.current_thread.lock().await.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("long query must have started");
        engine::cancel(execution.clone()).await.unwrap();
        engine::cancel(execution.clone()).await.unwrap();
        let (mut conn, cancelled) = tokio::time::timeout(Duration::from_secs(8), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(cancelled.cancelled);
        assert!(cancelled.session_valid);
        assert!(conn
            .query_first::<Option<String>, _>("SELECT @cancelled_tail")
            .await
            .unwrap()
            .unwrap()
            .is_none());
        // A late cancellation belongs to its original execution, never the next run.
        engine::cancel(execution).await.unwrap();
        let (report, outcomes) = run(&mut conn, &options, "SELECT 99;", 10).await;
        assert!(report.error.is_none());
        assert_eq!(outcomes[0].results[0].rows[0][0], integer(99));
        conn.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn authentication_tls_and_permissions_are_enforced() {
    for port in ports() {
        let p = profile(port);
        let wrong =
            database::options(&p, "incorrect-integration-password", Some("dbm_test")).unwrap();
        assert!(database::new_connection(wrong).await.is_err());
        let mut tls = p.clone();
        tls.tls = true;
        // The fixture certificate is deliberately untrusted without its CA.
        let verified_tls = database::options(&tls, &password(), Some("dbm_test")).unwrap();
        assert!(database::new_connection(verified_tls).await.is_err());
        tls.ca_pem = Some(
            std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../tests/fixtures/tls/ca.pem"),
            )
            .expect("the opt-in TLS test requires tests/fixtures/tls/ca.pem"),
        );
        let verified_tls = database::options(&tls, &password(), Some("dbm_test")).unwrap();
        let mut encrypted = database::new_connection(verified_tls).await.unwrap();
        let cipher: Option<(String, String)> = encrypted
            .query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
            .await
            .unwrap();
        assert!(!cipher.unwrap().1.is_empty());
        encrypted.disconnect().await.unwrap();
        // Correct trust does not bypass hostname validation. The certificate
        // has an IP SAN only, so localhost must be rejected as a DNS hostname.
        tls.host = "localhost".into();
        let mismatch = database::options(&tls, &password(), Some("dbm_test")).unwrap();
        let mismatch = database::new_connection(mismatch).await.err().unwrap();
        assert!(
            mismatch
                .message
                .to_ascii_lowercase()
                .contains("certificate")
                || mismatch.message.to_ascii_lowercase().contains("host name")
                || mismatch.message.to_ascii_lowercase().contains("hostname"),
            "TLS hostname mismatch should be reported: {}",
            mismatch.message
        );
        let mut reader = p;
        reader.username = "dbm_reader".into();
        let options = database::options(&reader, &password(), Some("dbm_test")).unwrap();
        let mut conn = database::new_connection(options).await.unwrap();
        assert!(!database::page(&mut conn, &request("composite_pk"))
            .await
            .unwrap()
            .result
            .rows
            .is_empty());
        let denied = database::apply(
            &mut conn,
            &change(
                "composite_pk",
                vec![RowChange {
                    kind: "insert".into(),
                    original: None,
                    values: vec![Some(integer(99)), Some(integer(99)), Some(text("denied"))],
                }],
            ),
        )
        .await
        .err()
        .unwrap();
        assert!(denied.code.starts_with("MYSQL_"));
        conn.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn stored_procedure_result_sets_and_deferred_errors_are_drained() {
    for port in ports() {
        let options = opts(port);
        let mut conn = database::new_connection(options.clone()).await.unwrap();
        let success = format!("success_{}", uuid::Uuid::new_v4().simple());
        let failure = format!("failure_{}", uuid::Uuid::new_v4().simple());
        // Definition goes directly through the driver, because the user-facing
        // script parser intentionally does not support DELIMITER in version 1.
        conn.query_drop(format!("CREATE PROCEDURE `{success}`() BEGIN SELECT 1 AS first_result; SELECT 2 AS second_result; END")).await.unwrap();
        conn.query_drop(format!("CREATE PROCEDURE `{failure}`() BEGIN SELECT 1 AS before_error; SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='intentional deferred failure'; END")).await.unwrap();
        let (report, outcomes) = run(
            &mut conn,
            &options,
            &format!("CALL `{success}`(); SELECT 3 AS after_call;"),
            10,
        )
        .await;
        assert!(report.error.is_none());
        assert_eq!(report.completed, 2);
        assert_eq!(outcomes[0].results.len(), 2);
        assert_eq!(outcomes[0].results[0].rows[0][0], integer(1));
        assert_eq!(outcomes[0].results[1].rows[0][0], integer(2));
        assert_eq!(outcomes[1].results[0].rows[0][0], integer(3));
        let (report, outcomes) = run(
            &mut conn,
            &options,
            &format!("BEGIN; CALL `{failure}`(); SET @deferred_tail=1;"),
            10,
        )
        .await;
        assert_eq!(report.completed, 1);
        assert!(report.session_valid);
        assert!(report.in_transaction);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(report.error.as_ref().unwrap().code, "MYSQL_1644");
        assert_eq!(
            report.error.as_ref().unwrap().sql_state.as_deref(),
            Some("45000")
        );
        assert!(conn
            .query_first::<Option<String>, _>("SELECT @deferred_tail")
            .await
            .unwrap()
            .unwrap()
            .is_none());
        conn.query_drop("ROLLBACK").await.unwrap();
        assert!(!engine::snapshot(&mut conn).await.unwrap().in_transaction);
        conn.query_drop(format!("DROP PROCEDURE `{success}`"))
            .await
            .unwrap();
        conn.query_drop(format!("DROP PROCEDURE `{failure}`"))
            .await
            .unwrap();
        conn.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn interrupted_sessions_are_invalidated_without_replaying_sql() {
    for port in ports() {
        let options = opts(port);
        let mut conn = database::new_connection(options.clone()).await.unwrap();
        let mut control = database::new_connection(options.clone()).await.unwrap();
        let execution = Arc::new(Execution::new("integration".into(), options));
        let runner_execution = execution.clone();
        let handle = tokio::spawn(async move {
            let report = engine::execute(
                &mut conn,
                "disconnect".into(),
                "SELECT SLEEP(30); SET @must_not_replay=1;",
                10,
                runner_execution,
                |_| {},
            )
            .await
            .unwrap();
            (conn, report)
        });
        let thread = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(thread) = *execution.current_thread.lock().await {
                    break thread;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        control
            .query_drop(format!("KILL CONNECTION {thread}"))
            .await
            .unwrap();
        let (conn, report) = tokio::time::timeout(Duration::from_secs(8), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(!report.session_valid);
        assert_eq!(report.completed, 0);
        assert!(report.error.is_some());
        let mut dead_session = Some(conn);
        assert!(!engine::close_requires_rollback(&mut dead_session).await);
        assert!(dead_session.is_none());
        let mut live_session = Some(database::new_connection(opts(port)).await.unwrap());
        live_session
            .as_mut()
            .unwrap()
            .query_drop("START TRANSACTION")
            .await
            .unwrap();
        assert!(engine::close_requires_rollback(&mut live_session).await);
        assert!(live_session.is_some());
        live_session
            .as_mut()
            .unwrap()
            .query_drop("ROLLBACK")
            .await
            .unwrap();
        assert!(!engine::close_requires_rollback(&mut live_session).await);
        live_session.take().unwrap().disconnect().await.unwrap();
        control.disconnect().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires disposable Docker databases; see docker-compose.test.yml"]
async fn failed_cancel_invalidates_session_even_if_query_finishes_first() {
    for port in ports() {
        let mut conn = database::new_connection(opts(port)).await.unwrap();
        // The read-only user cannot KILL a thread owned by dbm. Delaying control
        // connection initialization makes the query finish while cancellation
        // still owns the thread boundary, exercising the completion race.
        let control_options = OptsBuilder::from_opts(opts(port))
            .user(Some("dbm_reader"))
            .init(vec!["DO SLEEP(0.2)"])
            .into();
        let execution = Arc::new(Execution::new("integration".into(), control_options));
        let runner = execution.clone();
        let handle = tokio::spawn(async move {
            let report = engine::execute(
                &mut conn,
                "failed-cancel".into(),
                "SELECT SLEEP(0.05); SET @unsafe_tail=1;",
                10,
                runner,
                |_| {},
            )
            .await
            .unwrap();
            (conn, report)
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if execution.current_thread.lock().await.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            engine::cancel(execution).await.err().unwrap().code,
            "CANCEL_FAILED"
        );
        let (conn, report) = tokio::time::timeout(Duration::from_secs(8), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(report.cancelled);
        assert!(!report.session_valid);
        assert_eq!(report.error.unwrap().code, "SESSION_LOST");
        // An uncertain connection must be destroyed rather than queried again.
        drop(conn);
    }
}
