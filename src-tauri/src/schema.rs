//! Adds, edits, or removes a field or index against a fresh table definition.
use crate::{database, model::*, script::SqlMode};
use mysql_async::{prelude::Queryable, Conn};

fn invalid(message: &str) -> AppError {
    AppError::new("INVALID_DEFINITION", message)
}

// Record punctuation outside quotes and ordinary comments, including executable
// version comments. Offsets refer to the original UTF-8 string.
fn punctuation(source: &str, mode: SqlMode) -> Result<Vec<(usize, u8, usize)>> {
    fn scan(
        bytes: &[u8],
        start: usize,
        end: usize,
        mode: SqlMode,
        depth: &mut usize,
        marks: &mut Vec<(usize, u8, usize)>,
    ) -> Result<()> {
        let mut i = start;
        while i < end {
            let b = bytes[i];
            if matches!(b, b'\'' | b'"' | b'`') {
                let quote = b;
                let backslash = quote != b'`'
                    && !(quote == b'"' && mode.ansi_quotes)
                    && !mode.no_backslash_escapes;
                i += 1;
                let mut closed = false;
                while i < end {
                    if bytes[i] == b'\\' && backslash {
                        i += 2;
                    } else if bytes[i] == quote {
                        if i + 1 < end && bytes[i + 1] == quote {
                            i += 2;
                        } else {
                            i += 1;
                            closed = true;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                if !closed {
                    return Err(invalid("定义中存在未闭合的引号。"));
                }
                continue;
            }
            if b == b'/' && i + 1 < end && bytes[i + 1] == b'*' {
                let close = bytes[i + 2..end]
                    .windows(2)
                    .position(|v| v == b"*/")
                    .map(|at| i + 2 + at)
                    .ok_or_else(|| invalid("定义中存在未闭合的注释。"))?;
                if bytes.get(i + 2) == Some(&b'!') {
                    let mut body = i + 3;
                    while body < close && bytes[body].is_ascii_digit() {
                        body += 1;
                    }
                    scan(bytes, body, close, mode, depth, marks)?;
                }
                i = close + 2;
                continue;
            }
            if b == b'#'
                || (b == b'-'
                    && i + 2 < end
                    && bytes[i + 1] == b'-'
                    && bytes[i + 2].is_ascii_whitespace())
            {
                while i < end && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            match b {
                b'(' => {
                    marks.push((i, b, *depth));
                    *depth += 1;
                }
                b')' => {
                    *depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("定义中的括号不匹配。"))?;
                    marks.push((i, b, *depth));
                }
                b',' | b';' => marks.push((i, b, *depth)),
                _ => (),
            }
            i += 1;
        }
        Ok(())
    }
    let mut marks = Vec::new();
    let mut depth = 0;
    scan(
        source.as_bytes(),
        0,
        source.len(),
        mode,
        &mut depth,
        &mut marks,
    )?;
    if depth != 0 {
        return Err(invalid("定义中的括号不匹配。"));
    }
    Ok(marks)
}

fn entries(source: &str, mode: SqlMode) -> Result<Vec<&str>> {
    let marks = punctuation(source, mode)?;
    let start = marks
        .iter()
        .find(|(_, b, depth)| *b == b'(' && *depth == 0)
        .map(|(at, _, _)| at + 1)
        .ok_or_else(|| invalid("无法读取表定义。"))?;
    let end = marks
        .iter()
        .find(|(at, b, depth)| *at >= start && *b == b')' && *depth == 0)
        .map(|(at, _, _)| *at)
        .ok_or_else(|| invalid("无法读取表定义。"))?;
    let mut result = Vec::new();
    let mut previous = start;
    for (at, b, depth) in &marks {
        if *at >= start && *at < end && *b == b',' && *depth == 1 {
            result.push(source[previous..*at].trim());
            previous = at + 1;
        }
    }
    result.push(source[previous..end].trim());
    Ok(result)
}

fn named_definition<'a>(source: &'a str, name: &str, mode: SqlMode) -> Result<Option<&'a str>> {
    let quoted = database::quote_identifier(name)?;
    let rest = source.strip_prefix(&quoted).or_else(|| {
        mode.ansi_quotes
            .then(|| format!("\"{}\"", name.replace('"', "\"\"")))
            .and_then(|quoted| source.strip_prefix(&quoted))
    });
    Ok(rest
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map(str::trim))
}

fn extract(info: &TableInfo, target: &SchemaTarget, mode: SqlMode) -> Result<SchemaDefinition> {
    let exists = match target.kind.as_str() {
        "field" => info
            .fields
            .iter()
            .any(|field| field.name == target.original_name),
        "index" => info
            .indexes
            .iter()
            .any(|index| index.name == target.original_name),
        _ => return Err(invalid("请选择字段或索引。")),
    };
    if !exists {
        return Err(AppError::new(
            "NOT_FOUND",
            "字段或索引不存在，请刷新表结构。",
        ));
    }
    for entry in entries(&info.create_sql, mode)? {
        if target.kind == "field" {
            if let Some(definition) = named_definition(entry, &target.original_name, mode)? {
                return Ok(SchemaDefinition {
                    name: target.original_name.clone(),
                    definition: definition.into(),
                    index_kind: None,
                });
            }
        } else if target.kind == "index" {
            for (prefix, kind) in [
                ("PRIMARY KEY ", "primary"),
                ("UNIQUE KEY ", "unique"),
                ("FULLTEXT KEY ", "fulltext"),
                ("SPATIAL KEY ", "spatial"),
                ("KEY ", "index"),
            ] {
                if let Some(rest) = entry.strip_prefix(prefix) {
                    let definition = if kind == "primary" && target.original_name == "PRIMARY" {
                        Some(rest.trim())
                    } else if kind != "primary" {
                        named_definition(rest, &target.original_name, mode)?
                    } else {
                        None
                    };
                    if let Some(definition) = definition {
                        return Ok(SchemaDefinition {
                            name: target.original_name.clone(),
                            definition: definition.into(),
                            index_kind: Some(kind.into()),
                        });
                    }
                }
            }
        }
    }
    Err(AppError::new(
        "NOT_FOUND",
        "字段或索引不存在，请刷新表结构。",
    ))
}

fn validate_target(info: &TableInfo, target: &SchemaTarget) -> Result<()> {
    if !matches!(target.kind.as_str(), "field" | "index") {
        return Err(invalid("请选择字段或索引。"));
    }
    if target.action == SchemaAction::Add && !target.original_name.is_empty() {
        return Err(invalid("新增字段或索引不能指定原名称。"));
    }
    if info.engine.is_none() {
        return Err(AppError::new(
            "READ_ONLY",
            "视图不支持新增、修改或删除字段和索引，请编辑视图 SQL。",
        ));
    }
    if info.create_sql != target.expected_create_sql {
        return Err(AppError::new(
            "SCHEMA_CONFLICT",
            "表结构已改变，请刷新后重新操作。",
        ));
    }
    Ok(())
}

fn initial_definition(info: &TableInfo, target: &SchemaTarget) -> Result<SchemaDefinition> {
    let (definition, index_kind) = if target.kind == "field" {
        ("varchar(255) DEFAULT NULL".into(), None)
    } else {
        let field = info
            .fields
            .first()
            .ok_or_else(|| invalid("表中没有可用于索引的字段。"))?;
        (
            format!("({})", database::quote_identifier(&field.name)?),
            Some("index".into()),
        )
    };
    Ok(SchemaDefinition {
        name: String::new(),
        definition,
        index_kind,
    })
}

async fn current(conn: &mut Conn, target: &SchemaTarget) -> Result<(TableInfo, SqlMode)> {
    let mode: Option<String> = conn.query_first("SELECT @@SESSION.sql_mode").await?;
    let mode = SqlMode::parse(&mode.unwrap_or_default());
    let info = database::describe(conn, &target.database, &target.table).await?;
    validate_target(&info, target)?;
    Ok((info, mode))
}

pub async fn definition(conn: &mut Conn, target: &SchemaTarget) -> Result<SchemaDefinition> {
    let (info, mode) = current(conn, target).await?;
    if target.action == SchemaAction::Add {
        initial_definition(&info, target)
    } else {
        extract(&info, target, mode)
    }
}

fn build(info: &TableInfo, request: &SchemaChangeRequest, mode: SqlMode) -> Result<String> {
    let target = &request.target;
    let value = &request.value;
    validate_target(info, target)?;
    let original = if target.action == SchemaAction::Add {
        None
    } else {
        Some(extract(info, target, mode)?)
    };
    let qualified = format!(
        "{}.{}",
        database::quote_identifier(&target.database)?,
        database::quote_identifier(&target.table)?
    );
    if target.action == SchemaAction::Drop {
        let old = database::quote_identifier(&target.original_name)?;
        let operation = if target.kind == "field" {
            if info.fields.len() <= 1 {
                return Err(invalid("不能删除表中的最后一个字段。"));
            }
            format!("DROP COLUMN {old}")
        } else if original.as_ref().and_then(|v| v.index_kind.as_deref()) == Some("primary") {
            "DROP PRIMARY KEY".into()
        } else {
            format!("DROP INDEX {old}")
        };
        // Delete SQL uses only the target verified against current metadata.
        // Form values cannot rename or otherwise broaden the operation.
        return Ok(format!("ALTER TABLE {qualified}\n  {operation}"));
    }
    let definition = value.definition.trim();
    if definition.is_empty() || definition.len() > 65_536 || definition.contains('\0') {
        return Err(invalid("请填写有效的定义（最多 64 KiB）。"));
    }
    if punctuation(definition, mode)?
        .iter()
        .any(|(_, b, depth)| *b == b';' || (*b == b',' && *depth == 0))
    {
        return Err(invalid(
            "一次只能操作一个字段或索引，定义中不能包含分号或额外的 ALTER 操作。",
        ));
    }
    let name = database::quote_identifier(&value.name)?;
    if original.as_ref().is_some_and(|original| {
        value.name == original.name
            && definition == original.definition
            && value.index_kind == original.index_kind
    }) {
        return Err(AppError::new("NO_CHANGES", "尚未修改字段或索引。"));
    }
    let operation = if target.kind == "field" {
        if value.index_kind.is_some() || !definition.starts_with(char::is_alphabetic) {
            return Err(invalid("字段定义应以 MySQL 数据类型开头。"));
        }
        if info.fields.iter().any(|field| {
            field.name.eq_ignore_ascii_case(&value.name)
                && (target.action == SchemaAction::Add || field.name != target.original_name)
        }) {
            return Err(invalid("此字段名称已存在。"));
        }
        if target.action == SchemaAction::Add {
            format!("ADD COLUMN {name} {definition}")
        } else {
            let old = database::quote_identifier(&target.original_name)?;
            format!("CHANGE COLUMN {old} {name} {definition}")
        }
    } else {
        if info.indexes.iter().any(|index| {
            index.name.eq_ignore_ascii_case(&value.name)
                && (target.action == SchemaAction::Add || index.name != target.original_name)
        }) {
            return Err(invalid("此索引名称已存在。"));
        }
        let kind = value.index_kind.as_deref().unwrap_or("");
        let prefix = match kind {
            "primary" if value.name == "PRIMARY" => "PRIMARY KEY".into(),
            "unique" => format!("UNIQUE INDEX {name}"),
            "index" => format!("INDEX {name}"),
            "fulltext" => format!("FULLTEXT INDEX {name}"),
            "spatial" => format!("SPATIAL INDEX {name}"),
            _ => return Err(invalid("索引类型或主键名称无效。")),
        };
        if kind != "primary" && value.name.eq_ignore_ascii_case("PRIMARY") {
            return Err(invalid("PRIMARY 名称仅用于主键。"));
        }
        // SHOW CREATE uses a leading key-part list; require that shape rather
        // than accepting a second SQL/ALTER clause as index input.
        if !definition.starts_with('(') {
            return Err(invalid("索引定义应以括号中的字段列表开头。"));
        }
        if target.action == SchemaAction::Add {
            format!("ADD {prefix} {definition}")
        } else if original.as_ref().is_some_and(|original| {
            value.index_kind == original.index_kind && definition == original.definition
        }) && target.original_name != "PRIMARY"
        {
            let old = database::quote_identifier(&target.original_name)?;
            format!("RENAME INDEX {old} TO {name}")
        } else {
            let drop = if target.original_name == "PRIMARY" {
                "DROP PRIMARY KEY".into()
            } else {
                let old = database::quote_identifier(&target.original_name)?;
                format!("DROP INDEX {old}")
            };
            format!("{drop},\n  ADD {prefix} {definition}")
        }
    };
    Ok(format!("ALTER TABLE {qualified}\n  {operation}"))
}

pub async fn preview(conn: &mut Conn, request: &SchemaChangeRequest) -> Result<String> {
    let (info, mode) = current(conn, &request.target).await?;
    build(&info, request, mode)
}

pub async fn apply(
    conn: &mut Conn,
    request: &SchemaChangeRequest,
    expected_sql: &str,
) -> Result<()> {
    let sql = preview(conn, request).await?;
    if sql != expected_sql {
        return Err(AppError::new(
            "SCHEMA_CONFLICT",
            "待执行 SQL 与预览不一致，请重新预览。",
        ));
    }
    // One DDL statement; never use a data-edit transaction or split DROP/ADD.
    conn.query_drop(sql).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (TableInfo, SchemaChangeRequest) {
        let info = TableInfo {
            fields: vec![
                FieldInfo { name: "a`b".into(), data_type: "enum('x,y','it\\'s')".into(), nullable: true, default_value: Some("x,y".into()), extra: String::new(), comment: "中文; (ok)".into() },
                FieldInfo { name: "id".into(), data_type: "int".into(), nullable: false, default_value: None, extra: String::new(), comment: String::new() },
            ],
            indexes: vec![
                IndexInfo { name: "PRIMARY".into(), unique: true, columns: vec!["id".into()] },
                IndexInfo { name: "idx".into(), unique: true, columns: vec!["a`b".into()] },
            ],
            foreign_keys: vec![], engine: Some("InnoDB".into()), key_columns: vec![], editable: false,
            create_sql: "CREATE TABLE `t` (\n  `a``b` enum('x,y','it\\'s') COLLATE utf8mb4_bin DEFAULT 'x,y' COMMENT '中文; (ok)',\n  `id` int NOT NULL,\n  PRIMARY KEY (`id`),\n  UNIQUE KEY `idx` (`a``b`(4) DESC) COMMENT 'keep' /*!80000 INVISIBLE */\n) ENGINE=InnoDB".into(),
        };
        let request = SchemaChangeRequest {
            target: SchemaTarget {
                connection_id: "local".into(),
                database: "d`b".into(),
                table: "t".into(),
                kind: "field".into(),
                action: SchemaAction::Modify,
                original_name: "a`b".into(),
                expected_create_sql: info.create_sql.clone(),
            },
            value: SchemaDefinition {
                name: "renamed".into(),
                definition: "varchar(80) COLLATE utf8mb4_bin DEFAULT 'x,y' COMMENT '中文; (ok)'"
                    .into(),
                index_kind: None,
            },
        };
        (info, request)
    }
    #[test]
    fn extracts_complete_definitions_and_quotes_names() {
        let (info, request) = fixture();
        let original = extract(&info, &request.target, SqlMode::default()).unwrap();
        assert!(original.definition.contains("enum('x,y','it\\'s')"));
        assert!(original.definition.ends_with("COMMENT '中文; (ok)'"));
        assert!(build(&info, &request, SqlMode::default())
            .unwrap()
            .contains("ALTER TABLE `d``b`.`t`\n  CHANGE COLUMN `a``b` `renamed` varchar(80)"));
    }
    #[test]
    fn preserves_index_options_and_uses_single_alter() {
        let (info, mut request) = fixture();
        request.target.kind = "index".into();
        request.target.original_name = "idx".into();
        request.value = extract(&info, &request.target, SqlMode::default()).unwrap();
        assert_eq!(request.value.index_kind.as_deref(), Some("unique"));
        assert!(request.value.definition.contains("(4) DESC"));
        assert!(request.value.definition.ends_with("/*!80000 INVISIBLE */"));
        request.value.name = "new`index".into();
        assert!(build(&info, &request, SqlMode::default())
            .unwrap()
            .contains("RENAME INDEX `idx` TO `new``index`"));
        request.value.index_kind = Some("index".into());
        let sql = build(&info, &request, SqlMode::default()).unwrap();
        assert!(sql.contains("DROP INDEX `idx`,\n  ADD INDEX `new``index`"));
        assert!(sql.contains("COMMENT 'keep' /*!80000 INVISIBLE */"));
    }
    #[test]
    fn rejects_extra_operations_even_in_executable_comments() {
        let (info, mut request) = fixture();
        for source in [
            "int, DROP COLUMN id",
            "int; DROP TABLE t",
            "int /*!80000 , DROP COLUMN id */",
            "int DEFAULT (1;2)",
            "int DEFAULT ('unclosed)",
            "int)",
        ] {
            request.value.definition = source.into();
            assert!(
                build(&info, &request, SqlMode::default()).is_err(),
                "{source}"
            );
        }
        request.value.definition = "enum('a,b','x;y') DEFAULT 'x;y'".into();
        assert!(build(&info, &request, SqlMode::default()).is_ok());
    }
    #[test]
    fn handles_sql_modes_and_generated_expressions() {
        let source = "CREATE TABLE `t` (`a` varchar(10) DEFAULT 'x\\', `g` int GENERATED ALWAYS AS ((length(`a`) + 1)) STORED)";
        assert_eq!(
            entries(source, SqlMode::parse("NO_BACKSLASH_ESCAPES"))
                .unwrap()
                .len(),
            2
        );
        assert!(entries(source, SqlMode::default()).is_err());
        assert!(punctuation("int /*!80023 INVISIBLE */", SqlMode::default())
            .unwrap()
            .is_empty());
        let (mut info, mut request) = fixture();
        info.create_sql = "CREATE TABLE \"t\" (\"a\"\"b\" int, KEY \"idx\" (\"a\"\"b\"))".into();
        request.target.original_name = "a\"b".into();
        info.fields[0].name = "a\"b".into();
        let mode = SqlMode::parse("ANSI_QUOTES,NO_BACKSLASH_ESCAPES");
        assert_eq!(
            extract(&info, &request.target, mode).unwrap().definition,
            "int"
        );
        request.target.kind = "index".into();
        request.target.original_name = "idx".into();
        assert_eq!(
            extract(&info, &request.target, mode).unwrap().definition,
            "(\"a\"\"b\")"
        );
    }
    #[test]
    fn rejects_no_change_and_invalid_primary_name() {
        let (info, mut request) = fixture();
        request.value = extract(&info, &request.target, SqlMode::default()).unwrap();
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap_err().code,
            "NO_CHANGES"
        );
        request.target.kind = "index".into();
        request.target.original_name = "PRIMARY".into();
        request.value = SchemaDefinition {
            name: "other".into(),
            definition: "(`a``b`)".into(),
            index_kind: Some("primary".into()),
        };
        assert!(build(&info, &request, SqlMode::default()).is_err());
    }

    #[test]
    fn adds_fields_and_indexes_with_quoted_identifiers() {
        let (info, mut request) = fixture();
        request.target.action = SchemaAction::Add;
        request.target.original_name.clear();
        let initial = initial_definition(&info, &request.target).unwrap();
        assert!(initial.name.is_empty());
        assert_eq!(initial.definition, "varchar(255) DEFAULT NULL");
        assert!(initial.index_kind.is_none());
        request.value.name = "new`field".into();
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap(),
            "ALTER TABLE `d``b`.`t`\n  ADD COLUMN `new``field` varchar(80) COLLATE utf8mb4_bin DEFAULT 'x,y' COMMENT '中文; (ok)'"
        );

        request.target.kind = "index".into();
        request.value = initial_definition(&info, &request.target).unwrap();
        assert_eq!(request.value.definition, "(`a``b`)");
        assert_eq!(request.value.index_kind.as_deref(), Some("index"));
        request.value.name = "new`index".into();
        for (kind, prefix) in [
            ("index", "INDEX"),
            ("unique", "UNIQUE INDEX"),
            ("fulltext", "FULLTEXT INDEX"),
            ("spatial", "SPATIAL INDEX"),
        ] {
            request.value.index_kind = Some(kind.into());
            assert_eq!(
                build(&info, &request, SqlMode::default()).unwrap(),
                format!("ALTER TABLE `d``b`.`t`\n  ADD {prefix} `new``index` (`a``b`)")
            );
        }
    }

    #[test]
    fn drops_only_the_existing_target_and_protects_last_field() {
        let (mut info, mut request) = fixture();
        request.target.action = SchemaAction::Drop;
        request.value.name = "untrusted_name".into();
        request.value.definition = "int, DROP TABLE t".into();
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap(),
            "ALTER TABLE `d``b`.`t`\n  DROP COLUMN `a``b`"
        );
        info.fields.pop();
        assert!(build(&info, &request, SqlMode::default()).is_err());

        request.target.kind = "index".into();
        request.target.original_name = "idx".into();
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap(),
            "ALTER TABLE `d``b`.`t`\n  DROP INDEX `idx`"
        );
        request.target.original_name = "PRIMARY".into();
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap(),
            "ALTER TABLE `d``b`.`t`\n  DROP PRIMARY KEY"
        );
    }

    #[test]
    fn rejects_duplicate_add_names_and_duplicate_primary() {
        let (mut info, mut request) = fixture();
        request.target.action = SchemaAction::Add;
        request.target.original_name.clear();
        request.value.name = "A`B".into();
        assert!(build(&info, &request, SqlMode::default()).is_err());
        request.target.kind = "index".into();
        request.value = SchemaDefinition {
            name: "IDX".into(),
            definition: "(`id`)".into(),
            index_kind: Some("index".into()),
        };
        assert!(build(&info, &request, SqlMode::default()).is_err());
        request.value.name = "PRIMARY".into();
        request.value.index_kind = Some("primary".into());
        assert!(build(&info, &request, SqlMode::default()).is_err());
        info.indexes.retain(|index| index.name != "PRIMARY");
        assert_eq!(
            build(&info, &request, SqlMode::default()).unwrap(),
            "ALTER TABLE `d``b`.`t`\n  ADD PRIMARY KEY (`id`)"
        );
        request.value.name = "primary".into();
        assert!(build(&info, &request, SqlMode::default()).is_err());
        request.value.name = "PRIMARY".into();
        request.value.index_kind = Some("index".into());
        assert!(build(&info, &request, SqlMode::default()).is_err());
    }

    #[test]
    fn rejects_missing_unknown_or_inconsistent_targets() {
        let (info, mut request) = fixture();
        for action in [SchemaAction::Modify, SchemaAction::Drop] {
            request.target.action = action;
            request.target.original_name = "missing".into();
            assert_eq!(
                build(&info, &request, SqlMode::default()).unwrap_err().code,
                "NOT_FOUND"
            );
        }
        request.target.action = SchemaAction::Add;
        assert!(build(&info, &request, SqlMode::default()).is_err());
        request.target.original_name.clear();
        request.target.kind = "foreignKey".into();
        assert!(build(&info, &request, SqlMode::default()).is_err());
    }

    #[test]
    fn rejects_stale_schema_and_views_for_every_action() {
        let (mut info, mut request) = fixture();
        for action in [SchemaAction::Add, SchemaAction::Modify, SchemaAction::Drop] {
            request.target.action = action;
            request.target.original_name = if action == SchemaAction::Add {
                String::new()
            } else {
                "a`b".into()
            };
            request.target.expected_create_sql = "stale".into();
            assert_eq!(
                build(&info, &request, SqlMode::default()).unwrap_err().code,
                "SCHEMA_CONFLICT"
            );
            request.target.expected_create_sql = info.create_sql.clone();
            info.engine = None;
            assert_eq!(
                build(&info, &request, SqlMode::default()).unwrap_err().code,
                "READ_ONLY"
            );
            info.engine = Some("InnoDB".into());
        }
    }

    #[test]
    fn rejects_unsafe_added_definitions() {
        let (info, mut request) = fixture();
        request.target.action = SchemaAction::Add;
        request.target.original_name.clear();
        for definition in [
            "int, DROP COLUMN id",
            "int; DROP TABLE t",
            "int /*!80000 , DROP COLUMN id */",
            "int DEFAULT ('unclosed)",
            "int)",
            "",
        ] {
            request.value.definition = definition.into();
            assert!(build(&info, &request, SqlMode::default()).is_err());
        }
        request.target.kind = "index".into();
        request.value.index_kind = Some("index".into());
        for definition in [
            "(`id`), DROP COLUMN id",
            "(`id`) /*!80000 , DROP COLUMN id */",
            "(`id`); DROP TABLE t",
            "id",
        ] {
            request.value.definition = definition.into();
            assert!(build(&info, &request, SqlMode::default()).is_err());
        }
    }

    #[test]
    fn defaults_legacy_requests_to_modify_and_rejects_unknown_actions() {
        let (_, request) = fixture();
        let mut value = serde_json::to_value(&request.target).unwrap();
        assert_eq!(value["action"], "modify");
        value.as_object_mut().unwrap().remove("action");
        let target: SchemaTarget = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(target.action, SchemaAction::Modify);
        for action in ["add", "drop"] {
            value["action"] = action.into();
            assert!(serde_json::from_value::<SchemaTarget>(value.clone()).is_ok());
        }
        for action in ["delete", "ADD", ""] {
            value["action"] = action.into();
            assert!(serde_json::from_value::<SchemaTarget>(value.clone()).is_err());
        }
    }
}
