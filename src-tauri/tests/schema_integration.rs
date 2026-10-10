//! Opt-in schema edits in a randomly named database, never existing user tables.
//! DBM_SCHEMA_TEST_USER=root DBM_SCHEMA_TEST_PASSWORD=... cargo test --test schema_integration -- --ignored
use dbm_lib::{database, model::*, schema};
use mysql_async::{prelude::Queryable, Conn, Opts, OptsBuilder};

async fn target(conn: &mut Conn, database_name: &str, kind: &str, name: &str) -> SchemaTarget {
    action_target(
        conn,
        database_name,
        "edits",
        kind,
        name,
        SchemaAction::Modify,
    )
    .await
}

async fn action_target(
    conn: &mut Conn,
    database_name: &str,
    table: &str,
    kind: &str,
    name: &str,
    action: SchemaAction,
) -> SchemaTarget {
    SchemaTarget {
        connection_id: "schema-integration".into(),
        database: database_name.into(),
        table: table.into(),
        kind: kind.into(),
        action,
        original_name: name.into(),
        expected_create_sql: database::describe(conn, database_name, table)
            .await
            .unwrap()
            .create_sql,
    }
}
async fn execute(conn: &mut Conn, request: &SchemaChangeRequest) {
    let sql = schema::preview(conn, request).await.unwrap();
    schema::apply(conn, request, &sql).await.unwrap();
}

async fn add_request(
    conn: &mut Conn,
    db: &str,
    kind: &str,
    name: &str,
    definition: &str,
    index_kind: Option<&str>,
) -> SchemaChangeRequest {
    let target = action_target(conn, db, "lifecycle", kind, "", SchemaAction::Add).await;
    let mut value = schema::definition(conn, &target).await.unwrap();
    assert!(value.name.is_empty());
    assert!(!value.definition.is_empty());
    value.name = name.into();
    value.definition = definition.into();
    value.index_kind = index_kind.map(str::to_owned);
    SchemaChangeRequest { target, value }
}

async fn drop_request(
    conn: &mut Conn,
    db: &str,
    table: &str,
    kind: &str,
    name: &str,
) -> SchemaChangeRequest {
    let target = action_target(conn, db, table, kind, name, SchemaAction::Drop).await;
    let value = schema::definition(conn, &target).await.unwrap();
    SchemaChangeRequest { target, value }
}

async fn exercise_add_and_drop(conn: &mut Conn, db: &str) {
    conn.query_drop("CREATE TABLE lifecycle_parent (id INT NOT NULL PRIMARY KEY) ENGINE=InnoDB")
        .await
        .unwrap();
    conn.query_drop("INSERT INTO lifecycle_parent VALUES (1)")
        .await
        .unwrap();
    conn.query_drop("CREATE TABLE lifecycle (id INT NOT NULL PRIMARY KEY, parent_id INT, dependency_source INT, derived INT GENERATED ALWAYS AS (dependency_source + 1) STORED, KEY idx_parent (parent_id), CONSTRAINT fk_parent FOREIGN KEY (parent_id) REFERENCES lifecycle_parent(id)) ENGINE=InnoDB")
        .await
        .unwrap();
    conn.query_drop("INSERT INTO lifecycle (id, parent_id, dependency_source) VALUES (1, 1, 2)")
        .await
        .unwrap();

    let field = add_request(
        conn,
        db,
        "field",
        "extra`value",
        "VARCHAR(30) NULL DEFAULT 'quoted''value,x' COMMENT '新增字段'",
        None,
    )
    .await;
    let sql = schema::preview(conn, &field).await.unwrap();
    assert!(sql.contains("ADD COLUMN `extra``value`"));
    // Confirmation is bound to the exact preview and may not substitute SQL.
    assert_eq!(
        schema::apply(conn, &field, "different SQL")
            .await
            .err()
            .unwrap()
            .code,
        "SCHEMA_CONFLICT"
    );
    assert!(!database::describe(conn, db, "lifecycle")
        .await
        .unwrap()
        .fields
        .iter()
        .any(|field| field.name == "extra`value"));
    schema::apply(conn, &field, &sql).await.unwrap();
    let info = database::describe(conn, db, "lifecycle").await.unwrap();
    let added = info
        .fields
        .iter()
        .find(|field| field.name == "extra`value")
        .unwrap();
    assert_eq!(added.data_type, "varchar(30)");
    assert!(added.nullable);
    assert_eq!(added.default_value.as_deref(), Some("quoted'value,x"));
    assert_eq!(added.comment, "新增字段");
    let default_value: Option<String> = conn
        .query_first("SELECT `extra``value` FROM lifecycle WHERE id = 1")
        .await
        .unwrap();
    assert_eq!(default_value.as_deref(), Some("quoted'value,x"));
    // An add dialog opened against an earlier table snapshot must refresh.
    assert_eq!(
        schema::preview(conn, &field).await.err().unwrap().code,
        "SCHEMA_CONFLICT"
    );

    let duplicate = add_request(conn, db, "field", "extra`value", "INT NULL", None).await;
    assert!(schema::preview(conn, &duplicate).await.is_err());
    assert_eq!(
        database::describe(conn, db, "lifecycle")
            .await
            .unwrap()
            .create_sql,
        info.create_sql
    );

    for (name, definition, kind) in [
        ("idx`extra", "(`extra``value`(10) DESC)", "index"),
        ("unique_extra", "(`extra``value`)", "unique"),
        (
            "composite_extra",
            "(`extra``value`, `parent_id` DESC)",
            "index",
        ),
        ("expression_extra", "((length(`extra``value`)))", "index"),
    ] {
        let request = add_request(conn, db, "index", name, definition, Some(kind)).await;
        let sql = schema::preview(conn, &request).await.unwrap();
        assert!(sql.contains("ADD "));
        assert!(!sql.contains("DROP "));
        schema::apply(conn, &request, &sql).await.unwrap();
        let info = database::describe(conn, db, "lifecycle").await.unwrap();
        let added = info
            .indexes
            .iter()
            .find(|index| index.name == name)
            .unwrap();
        assert_eq!(added.unique, kind == "unique");
        let definition_target =
            action_target(conn, db, "lifecycle", "index", name, SchemaAction::Modify).await;
        let stored = schema::definition(conn, &definition_target).await.unwrap();
        if name == "idx`extra" {
            assert!(stored.definition.contains("(10) DESC"));
        } else if name == "composite_extra" {
            assert_eq!(added.columns, ["extra`value", "parent_id"]);
            assert!(stored.definition.contains("DESC"));
        } else if name == "expression_extra" {
            assert!(stored.definition.contains("length("));
        }
    }
    let duplicate = add_request(
        conn,
        db,
        "index",
        "idx`extra",
        "(`parent_id`)",
        Some("index"),
    )
    .await;
    let before = database::describe(conn, db, "lifecycle").await.unwrap();
    assert!(schema::preview(conn, &duplicate).await.is_err());
    assert_eq!(
        database::describe(conn, db, "lifecycle")
            .await
            .unwrap()
            .create_sql,
        before.create_sql
    );

    // MySQL rejects removal of a generated column's source or a foreign key's
    // required index. Failed DDL must leave the entire table definition intact.
    for (kind, name) in [("field", "dependency_source"), ("index", "idx_parent")] {
        let request = drop_request(conn, db, "lifecycle", kind, name).await;
        let sql = schema::preview(conn, &request).await.unwrap();
        let error = schema::apply(conn, &request, &sql).await.err().unwrap();
        assert!(error.code.starts_with("MYSQL_"));
        assert_eq!(
            database::describe(conn, db, "lifecycle")
                .await
                .unwrap()
                .create_sql,
            before.create_sql
        );
    }

    let stale_drop = drop_request(conn, db, "lifecycle", "field", "extra`value").await;
    for name in [
        "idx`extra",
        "unique_extra",
        "composite_extra",
        "expression_extra",
    ] {
        let request = drop_request(conn, db, "lifecycle", "index", name).await;
        let sql = schema::preview(conn, &request).await.unwrap();
        assert!(sql.contains("DROP INDEX"));
        assert!(!sql.contains("ADD "));
        schema::apply(conn, &request, &sql).await.unwrap();
        assert!(!database::describe(conn, db, "lifecycle")
            .await
            .unwrap()
            .indexes
            .iter()
            .any(|index| index.name == name));
    }
    assert_eq!(
        schema::preview(conn, &stale_drop).await.err().unwrap().code,
        "SCHEMA_CONFLICT"
    );
    let request = drop_request(conn, db, "lifecycle", "field", "extra`value").await;
    let sql = schema::preview(conn, &request).await.unwrap();
    assert!(sql.contains("DROP COLUMN `extra``value`"));
    assert_eq!(
        schema::apply(conn, &request, "different SQL")
            .await
            .err()
            .unwrap()
            .code,
        "SCHEMA_CONFLICT"
    );
    schema::apply(conn, &request, &sql).await.unwrap();
    assert!(!database::describe(conn, db, "lifecycle")
        .await
        .unwrap()
        .fields
        .iter()
        .any(|field| field.name == "extra`value"));

    // This primary key has no AUTO_INCREMENT column, so removing it is valid.
    conn.query_drop(
        "CREATE TABLE primary_drop (id INT NOT NULL PRIMARY KEY, payload INT) ENGINE=InnoDB",
    )
    .await
    .unwrap();
    let request = drop_request(conn, db, "primary_drop", "index", "PRIMARY").await;
    let sql = schema::preview(conn, &request).await.unwrap();
    assert!(sql.contains("DROP PRIMARY KEY"));
    schema::apply(conn, &request, &sql).await.unwrap();
    assert!(database::describe(conn, db, "primary_drop")
        .await
        .unwrap()
        .indexes
        .is_empty());
}

async fn exercise(conn: &mut Conn, db: &str) {
    conn.query_drop(format!("USE `{db}`")).await.unwrap();
    conn.query_drop("CREATE TABLE edits (id INT NOT NULL AUTO_INCREMENT, label VARCHAR(80) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL DEFAULT 'x,y' COMMENT '中文', stamp TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3), source_label VARCHAR(80), `generated` INT GENERATED ALWAYS AS (length(source_label)) STORED, PRIMARY KEY (id), KEY idx_label (label(12) DESC) COMMENT 'keep' INVISIBLE) ENGINE=InnoDB").await.unwrap();
    conn.query_drop("INSERT INTO edits(label) VALUES ('before'), ('before')")
        .await
        .unwrap();

    let t = target(conn, db, "field", "label").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    assert!(value.definition.contains("COLLATE utf8mb4_bin"));
    value.name = "renamed`label".into();
    value.definition = value.definition.replace("varchar(80)", "varchar(100)");
    execute(
        conn,
        &SchemaChangeRequest {
            target: t.clone(),
            value,
        },
    )
    .await;
    let info = database::describe(conn, db, "edits").await.unwrap();
    assert!(info.fields.iter().any(|field| field.name == "renamed`label"
        && field.data_type == "varchar(100)"
        && field.comment == "中文"));
    assert!(info.create_sql.contains("COLLATE utf8mb4_bin"));
    assert!(info.create_sql.contains("ON UPDATE CURRENT_TIMESTAMP(3)"));
    assert!(info.create_sql.contains("GENERATED ALWAYS AS"));
    let dependent = target(conn, db, "field", "source_label").await;
    let mut value = schema::definition(conn, &dependent).await.unwrap();
    value.name = "source_renamed".into();
    let blocked = SchemaChangeRequest {
        target: dependent,
        value,
    };
    let sql = schema::preview(conn, &blocked).await.unwrap();
    assert!(schema::apply(conn, &blocked, &sql).await.is_err());
    assert!(database::describe(conn, db, "edits")
        .await
        .unwrap()
        .fields
        .iter()
        .any(|field| field.name == "source_label"));
    let generated = target(conn, db, "field", "generated").await;
    let mut value = schema::definition(conn, &generated).await.unwrap();
    value.name = "source_length".into();
    execute(
        conn,
        &SchemaChangeRequest {
            target: generated,
            value,
        },
    )
    .await;
    // A previously opened editor must not apply after a different schema edit.
    assert_eq!(
        schema::definition(conn, &t).await.err().unwrap().code,
        "SCHEMA_CONFLICT"
    );

    let t = target(conn, db, "field", "stamp").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    value.name = "updated_at".into();
    execute(conn, &SchemaChangeRequest { target: t, value }).await;
    assert!(database::describe(conn, db, "edits")
        .await
        .unwrap()
        .create_sql
        .contains("ON UPDATE CURRENT_TIMESTAMP(3)"));

    let t = target(conn, db, "index", "idx_label").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    assert!(value.definition.contains("(12) DESC"));
    assert!(value.definition.contains("COMMENT 'keep'"));
    assert!(value.definition.contains("INVISIBLE"));
    value.name = "idx_renamed".into();
    let request = SchemaChangeRequest { target: t, value };
    assert!(schema::preview(conn, &request)
        .await
        .unwrap()
        .contains("RENAME INDEX"));
    execute(conn, &request).await;

    let t = target(conn, db, "index", "idx_renamed").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    value.index_kind = Some("unique".into());
    let request = SchemaChangeRequest {
        target: t.clone(),
        value,
    };
    let sql = schema::preview(conn, &request).await.unwrap();
    assert!(sql.contains("DROP INDEX"));
    // Duplicate rows make the rebuilt UNIQUE index fail; the old index survives.
    assert!(schema::apply(conn, &request, &sql).await.is_err());
    assert!(database::describe(conn, db, "edits")
        .await
        .unwrap()
        .indexes
        .iter()
        .any(|index| index.name == "idx_renamed" && !index.unique));
    conn.query_drop("DELETE FROM edits WHERE id = 2")
        .await
        .unwrap();
    execute(conn, &request).await;
    let info = database::describe(conn, db, "edits").await.unwrap();
    assert!(info
        .indexes
        .iter()
        .any(|index| index.name == "idx_renamed" && index.unique));
    assert!(info.create_sql.contains("(12) DESC"));
    assert!(info.create_sql.contains("COMMENT 'keep'"));
    assert!(info.create_sql.contains("INVISIBLE"));

    conn.query_drop("ALTER TABLE edits ADD INDEX expression_idx ((length(`renamed``label`)))")
        .await
        .unwrap();
    let t = target(conn, db, "index", "expression_idx").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    assert!(value.definition.contains("(("));
    value.definition = "((length(`renamed``label`)), `id` DESC)".into();
    execute(conn, &SchemaChangeRequest { target: t, value }).await;

    let t = target(conn, db, "field", "id").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    value.definition = "bigint NOT NULL AUTO_INCREMENT COMMENT 'identifier'".into();
    let request = SchemaChangeRequest { target: t, value };
    let sql = schema::preview(conn, &request).await.unwrap();
    assert_eq!(
        schema::apply(conn, &request, "different SQL")
            .await
            .err()
            .unwrap()
            .code,
        "SCHEMA_CONFLICT"
    );
    schema::apply(conn, &request, &sql).await.unwrap();
    assert!(
        database::describe(conn, db, "edits").await.unwrap().fields[0]
            .extra
            .contains("auto_increment")
    );

    conn.query_drop("CREATE VIEW read_only_view AS SELECT id FROM edits")
        .await
        .unwrap();
    let mut t = target(conn, db, "field", "id").await;
    t.table = "read_only_view".into();
    t.expected_create_sql = database::describe(conn, db, "read_only_view")
        .await
        .unwrap()
        .create_sql;
    assert_eq!(
        schema::definition(conn, &t).await.err().unwrap().code,
        "READ_ONLY"
    );

    exercise_add_and_drop(conn, db).await;

    conn.query_drop("SET SESSION sql_mode = 'NO_BACKSLASH_ESCAPES,ANSI_QUOTES,STRICT_ALL_TABLES'")
        .await
        .unwrap();
    let t = target(conn, db, "field", "updated_at").await;
    let mut value = schema::definition(conn, &t).await.unwrap();
    value.name = "stamp_again".into();
    execute(conn, &SchemaChangeRequest { target: t, value }).await;
}

#[tokio::test]
#[ignore = "requires a local MySQL account permitted to create a disposable database"]
async fn complete_schema_edits_preserve_attributes_and_reject_conflicts() {
    let options: Opts = OptsBuilder::default()
        .ip_or_hostname(
            std::env::var("DBM_SCHEMA_TEST_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        )
        .tcp_port(
            std::env::var("DBM_SCHEMA_TEST_PORT")
                .unwrap_or_else(|_| "3306".into())
                .parse()
                .unwrap(),
        )
        .user(Some(
            std::env::var("DBM_SCHEMA_TEST_USER").unwrap_or_else(|_| "root".into()),
        ))
        .pass(Some(
            std::env::var("DBM_SCHEMA_TEST_PASSWORD").unwrap_or_default(),
        ))
        .prefer_socket(false)
        .into();
    let mut conn = Conn::new(options.clone()).await.unwrap();
    let version: String = conn.query_first("SELECT VERSION()").await.unwrap().unwrap();
    println!("Schema editing integration: MySQL {version}");
    let db = format!("dbm_schema_test_{}", uuid::Uuid::new_v4().simple());
    conn.query_drop(format!("CREATE DATABASE `{db}` CHARACTER SET utf8mb4"))
        .await
        .unwrap();
    // Always clean up our random schema, including when an assertion fails.
    let test_db = db.clone();
    let tested = tokio::spawn(async move {
        exercise(&mut conn, &test_db).await;
        conn
    })
    .await;
    match tested {
        Ok(mut conn) => {
            conn.query_drop(format!("DROP DATABASE `{db}`"))
                .await
                .unwrap();
            conn.disconnect().await.unwrap();
        }
        Err(error) => {
            let mut cleanup = Conn::new(options).await.unwrap();
            cleanup
                .query_drop(format!("DROP DATABASE `{db}`"))
                .await
                .unwrap();
            cleanup.disconnect().await.unwrap();
            std::panic::resume_unwind(error.into_panic())
        }
    }
}
