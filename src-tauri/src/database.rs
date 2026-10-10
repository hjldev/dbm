//! MySQL operations used by the desktop commands. Values never enter SQL text;
//! every identifier is quoted and every filter is checked against fresh metadata.
use crate::model::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use mysql_async::{
    consts::{ColumnFlags, ColumnType},
    prelude::Queryable,
    Column, Conn, Opts, OptsBuilder, PoolConstraints, PoolOpts, Row, SslOpts, TxOpts, Value,
};
use std::{collections::BTreeMap, time::Duration};

const DEFAULT_PAGE_SIZE: u32 = 200;
const MAX_PAGE_SIZE: u32 = 5000;
type ObjectMetadataRow = (String, String, Option<String>, Option<String>, String);

pub fn quote_identifier(value: &str) -> Result<String> {
    if value.is_empty() || value.chars().count() > 64 || value.contains('\0') {
        return Err(AppError::new(
            "INVALID_IDENTIFIER",
            "数据库、表或字段名称无效",
        ));
    }
    Ok(format!("`{}`", value.replace('`', "``")))
}

pub fn options(
    profile: &ConnectionProfile,
    password: &str,
    database: Option<&str>,
) -> Result<Opts> {
    if profile.name.trim().is_empty()
        || profile.name.chars().count() > 128
        || profile.name.contains('\0')
    {
        return Err(AppError::new(
            "INVALID_CONNECTION",
            "请填写连接名称（最多 128 个字符）",
        ));
    }
    if profile.host.is_empty()
        || profile.host.len() > 255
        || profile
            .host
            .chars()
            .any(|c| c.is_whitespace() || c == '\0' || c == '/')
    {
        return Err(AppError::new(
            "INVALID_CONNECTION",
            "请填写有效的主机名或 IP 地址",
        ));
    }
    if profile.port == 0
        || profile.username.is_empty()
        || profile.username.contains('\0')
        || profile.username.chars().count() > 128
        || password.contains('\0')
    {
        return Err(AppError::new("INVALID_CONNECTION", "端口或用户凭据无效"));
    }
    let database = database.filter(|v| !v.is_empty());
    if let Some(value) = database {
        quote_identifier(value)?;
    }
    let ssl = if profile.tls {
        let mut ssl = SslOpts::default(); // Both certificate and hostname verification stay enabled.
        if let Some(pem) = profile.ca_pem.as_ref().filter(|v| !v.is_empty()) {
            if pem.len() > 1024 * 1024 || !pem.contains("-----BEGIN CERTIFICATE-----") {
                return Err(AppError::new(
                    "INVALID_CA",
                    "CA 必须是有效的 PEM 证书，且大小不超过 1 MiB",
                ));
            }
            // Buffer input cannot be interpreted as an arbitrary filesystem path.
            ssl = ssl.with_root_certs(vec![pem.as_bytes().to_vec().into()]);
        }
        Some(ssl)
    } else {
        None
    };
    let pool = PoolOpts::default()
        .with_constraints(PoolConstraints::new(0, 2).expect("valid constant pool bounds"))
        .with_inactive_connection_ttl(Duration::from_secs(60))
        .with_ttl_check_interval(Duration::from_secs(10));
    Ok(OptsBuilder::default()
        .ip_or_hostname(profile.host.clone())
        .tcp_port(profile.port)
        .user(Some(profile.username.clone()))
        .pass(Some(password.to_owned()))
        .db_name(database.map(str::to_owned))
        .ssl_opts(ssl)
        .prefer_socket(false)
        .pool_opts(pool)
        .stmt_cache_size(16)
        // setup runs after COM_RESET_CONNECTION too; init runs only at connect.
        .setup(vec!["SET NAMES utf8mb4"])
        .into())
}

pub async fn new_connection(opts: Opts) -> Result<Conn> {
    tokio::time::timeout(Duration::from_secs(10), Conn::new(opts))
        .await
        .map_err(|_| {
            AppError::new(
                "CONNECT_TIMEOUT",
                "连接超时（10 秒），请检查地址、端口和网络",
            )
        })?
        .map_err(Into::into)
}

pub async fn databases(conn: &mut Conn) -> Result<Vec<String>> {
    Ok(conn.query("SHOW DATABASES").await?)
}

pub async fn objects(conn: &mut Conn, database: &str) -> Result<Vec<ObjectInfo>> {
    quote_identifier(database)?;
    let rows: Vec<ObjectMetadataRow> = conn.exec(
        "SELECT TABLE_NAME,TABLE_TYPE,ENGINE,CAST(TABLE_ROWS AS CHAR),TABLE_COMMENT FROM information_schema.TABLES WHERE TABLE_SCHEMA=? ORDER BY TABLE_NAME",
        (database,),
    ).await?;
    Ok(rows
        .into_iter()
        .map(|(name, kind, engine, estimated_rows, comment)| ObjectInfo {
            name,
            kind: if kind == "VIEW" {
                "view".into()
            } else {
                "table".into()
            },
            engine,
            estimated_rows,
            comment,
        })
        .collect())
}

pub async fn describe(conn: &mut Conn, database: &str, table: &str) -> Result<TableInfo> {
    describe_with(conn, database, table).await
}

async fn describe_with<Q: Queryable>(
    conn: &mut Q,
    database: &str,
    table: &str,
) -> Result<TableInfo> {
    let qualified = qualified_name(database, table)?;
    let object: Option<(String, Option<String>)> = conn.exec_first(
        "SELECT TABLE_TYPE,ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?", (database, table),
    ).await?;
    let (kind, engine) =
        object.ok_or_else(|| AppError::new("NOT_FOUND", "表或视图不存在，或当前用户无权访问"))?;
    let fields: Vec<(String, String, String, Option<String>, String, String)> = conn.exec(
        "SELECT COLUMN_NAME,COLUMN_TYPE,IS_NULLABLE,COLUMN_DEFAULT,EXTRA,COLUMN_COMMENT FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=? AND TABLE_NAME=? ORDER BY ORDINAL_POSITION", (database, table),
    ).await?;
    let fields: Vec<FieldInfo> = fields
        .into_iter()
        .map(
            |(name, data_type, nullable, default_value, extra, comment)| FieldInfo {
                name,
                data_type,
                nullable: nullable == "YES",
                default_value,
                extra,
                comment,
            },
        )
        .collect();
    let index_rows: Vec<(String, u8, Option<String>)> = conn.exec(
        "SELECT INDEX_NAME,NON_UNIQUE,COLUMN_NAME FROM information_schema.STATISTICS WHERE TABLE_SCHEMA=? AND TABLE_NAME=? ORDER BY INDEX_NAME,SEQ_IN_INDEX", (database, table),
    ).await?;
    let mut indexes: BTreeMap<String, IndexInfo> = BTreeMap::new();
    for (name, non_unique, column) in index_rows {
        let entry = indexes.entry(name.clone()).or_insert_with(|| IndexInfo {
            name,
            unique: non_unique == 0,
            columns: vec![],
        });
        if let Some(column) = column {
            entry.columns.push(column);
        } else {
            entry.unique = false;
        }
    }
    let indexes: Vec<IndexInfo> = indexes.into_values().collect();
    let mut key_candidates: Vec<&IndexInfo> = indexes
        .iter()
        .filter(|index| {
            index.unique
                && !index.columns.is_empty()
                && index
                    .columns
                    .iter()
                    .all(|name| fields.iter().any(|f| &f.name == name && !f.nullable))
        })
        .collect();
    key_candidates.sort_by_key(|index| (index.name != "PRIMARY", index.columns.len(), &index.name));
    let key_columns = key_candidates
        .first()
        .map(|i| i.columns.clone())
        .unwrap_or_default();
    let foreign_rows: Vec<(String, String, String, String, String)> = conn.exec(
        "SELECT CONSTRAINT_NAME,COLUMN_NAME,REFERENCED_TABLE_SCHEMA,REFERENCED_TABLE_NAME,REFERENCED_COLUMN_NAME FROM information_schema.KEY_COLUMN_USAGE WHERE TABLE_SCHEMA=? AND TABLE_NAME=? AND REFERENCED_TABLE_NAME IS NOT NULL ORDER BY CONSTRAINT_NAME,ORDINAL_POSITION", (database, table),
    ).await?;
    let foreign_keys = foreign_rows
        .into_iter()
        .map(
            |(name, column, referenced_schema, referenced_table, referenced_column)| {
                ForeignKeyInfo {
                    name,
                    column,
                    referenced_schema,
                    referenced_table,
                    referenced_column,
                }
            },
        )
        .collect();
    let create_row: Option<Row> = conn
        .query_first(format!(
            "SHOW CREATE {} {qualified}",
            if kind == "VIEW" { "VIEW" } else { "TABLE" }
        ))
        .await?;
    let create_sql = create_row
        .and_then(|row| row.get::<String, _>(1))
        .ok_or_else(|| AppError::new("METADATA", "无法读取建表 SQL"))?;
    let editable = engine
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("InnoDB"))
        && !key_columns.is_empty();
    Ok(TableInfo {
        fields,
        indexes,
        foreign_keys,
        create_sql,
        engine,
        key_columns,
        editable,
    })
}

fn qualified_name(database: &str, table: &str) -> Result<String> {
    Ok(format!(
        "{}.{}",
        quote_identifier(database)?,
        quote_identifier(table)?
    ))
}

fn field<'a>(info: &'a TableInfo, name: &str) -> Result<&'a FieldInfo> {
    info.fields.iter().find(|f| f.name == name).ok_or_else(|| {
        AppError::new(
            "INVALID_COLUMN",
            format!("字段 {name} 不存在，请刷新表结构"),
        )
    })
}

fn page_size(request: &PageRequest) -> Result<u32> {
    let size = if request.page_size == 0 {
        DEFAULT_PAGE_SIZE
    } else {
        request.page_size
    };
    if size > MAX_PAGE_SIZE {
        return Err(AppError::new("INVALID_PAGE", "每页最多 5,000 行"));
    }
    Ok(size)
}

pub fn table_query(
    info: &TableInfo,
    request: &PageRequest,
    paginate: bool,
) -> Result<(String, Vec<Value>)> {
    let table = qualified_name(&request.database, &request.table)?;
    if info.fields.is_empty() {
        return Err(AppError::new("METADATA", "表中没有可读取的字段"));
    }
    if request.filters.len() > 100 {
        return Err(AppError::new("INVALID_FILTER", "最多允许 100 个筛选条件"));
    }
    let columns = info
        .fields
        .iter()
        .map(|f| quote_identifier(&f.name))
        .collect::<Result<Vec<_>>>()?
        .join(",");
    let mut sql = format!("SELECT {columns} FROM {table}");
    let mut params = Vec::new();
    let mut clauses = Vec::new();
    for filter in &request.filters {
        field(info, &filter.column)?;
        let column = quote_identifier(&filter.column)?;
        let clause = match filter.operator.as_str() {
            "isNull" => format!("{column} IS NULL"),
            "isNotNull" => format!("{column} IS NOT NULL"),
            "eq" | "ne" | "gt" | "gte" | "lt" | "lte" => {
                let value = filter
                    .value
                    .as_ref()
                    .ok_or_else(|| AppError::new("INVALID_FILTER", "筛选条件缺少值"))?;
                if *value == CellValue::Null {
                    match filter.operator.as_str() {
                        "eq" => format!("{column} IS NULL"),
                        "ne" => format!("{column} IS NOT NULL"),
                        _ => {
                            return Err(AppError::new(
                                "INVALID_FILTER",
                                "NULL 只支持等于、不等于或空值筛选",
                            ))
                        }
                    }
                } else {
                    let operator = match filter.operator.as_str() {
                        "eq" => "=",
                        "ne" => "<>",
                        "gt" => ">",
                        "gte" => ">=",
                        "lt" => "<",
                        _ => "<=",
                    };
                    params.push(cell_param(value)?);
                    format!("{column} {operator} ?")
                }
            }
            "contains" => {
                let value = filter
                    .value
                    .as_ref()
                    .ok_or_else(|| AppError::new("INVALID_FILTER", "包含筛选缺少文本"))?;
                if matches!(value, CellValue::Null | CellValue::Binary(_)) {
                    return Err(AppError::new("INVALID_FILTER", "包含筛选需要文本值"));
                }
                // Explicit ! escape works with and without NO_BACKSLASH_ESCAPES.
                let escaped = value
                    .display()
                    .replace('!', "!!")
                    .replace('%', "!%")
                    .replace('_', "!_");
                params.push(Value::Bytes(format!("%{escaped}%").into_bytes()));
                format!("{column} LIKE ? ESCAPE '!'")
            }
            _ => return Err(AppError::new("INVALID_FILTER", "不支持此筛选运算符")),
        };
        clauses.push(clause);
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    let mut order = Vec::new();
    if let Some(sort) = &request.sort_column {
        field(info, sort)?;
        order.push(format!(
            "{} {}",
            quote_identifier(sort)?,
            if request.sort_desc { "DESC" } else { "ASC" }
        ));
    }
    for key in &info.key_columns {
        field(info, key)?;
        if request.sort_column.as_ref() != Some(key) {
            order.push(format!("{} ASC", quote_identifier(key)?));
        }
    }
    if !order.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order.join(","));
    }
    if paginate {
        let size = page_size(request)?;
        let offset = request
            .page
            .checked_mul(u64::from(size))
            .ok_or_else(|| AppError::new("INVALID_PAGE", "页码过大"))?;
        sql.push_str(" LIMIT ? OFFSET ?");
        params.push(Value::UInt(u64::from(size) + 1));
        params.push(Value::UInt(offset));
    }
    Ok((sql, params))
}

pub async fn page(conn: &mut Conn, request: &PageRequest) -> Result<PageResult> {
    let size = page_size(request)? as usize;
    let info = describe(conn, &request.database, &request.table).await?;
    let (sql, params) = table_query(&info, request, true)?;
    let mut query = conn.exec_iter(sql, params).await?;
    let columns = query.columns_ref().to_vec();
    let mut rows = Vec::with_capacity(size.min(200));
    let mut has_next = false;
    while let Some(row) = query.next().await? {
        if rows.len() == size {
            has_next = true;
        } else {
            rows.push(row_values(row, &columns)?);
        }
    }
    query.drop_result().await?;
    let total_rows = rows.len() as u64;
    Ok(PageResult {
        has_next,
        result: ResultSet {
            columns: columns.iter().map(column_meta).collect(),
            rows,
            total_rows,
            truncated: has_next,
        },
    })
}

pub async fn apply(conn: &mut Conn, request: &ChangeRequest) -> Result<u64> {
    if request.changes.is_empty() {
        return Ok(0);
    }
    if request.changes.len() > 5000 {
        return Err(AppError::new("INVALID_CHANGE", "每次最多提交 5,000 条变更"));
    }
    let table = qualified_name(&request.database, &request.table)?;
    let mut tx = conn.start_transaction(TxOpts::default()).await?;
    let operation: Result<u64> = async {
        // Acquire the table's metadata lock before inspecting its schema. DDL
        // cannot swap an engine or key definition underneath this transaction.
        tx.query_drop(format!("SELECT * FROM {table} LIMIT 0"))
            .await?;
        let info = describe_with(&mut tx, &request.database, &request.table).await?;
        if !info.editable {
            return Err(AppError::new(
                "READ_ONLY",
                "图形编辑需要 InnoDB 表及主键或非空唯一键",
            ));
        }
        let mut affected = 0;
        for change in &request.changes {
            validate_change(&info, change)?;
            if change.kind == "insert" {
                let (sql, params) = insert_query(&info, &table, change)?;
                tx.exec_drop(sql, params).await?;
                affected += tx.affected_rows();
                continue;
            }
            let original = change
                .original
                .as_ref()
                .ok_or_else(|| AppError::new("INVALID_CHANGE", "修改或删除需要完整的原始行"))?;
            let (predicate, params) = key_predicate(&info, original)?;
            let selection = info
                .fields
                .iter()
                .map(|f| quote_identifier(&f.name))
                .collect::<Result<Vec<_>>>()?
                .join(",");
            let matches: Vec<Row> = tx
                .exec(
                    format!("SELECT {selection} FROM {table} WHERE {predicate} LIMIT 2 FOR UPDATE"),
                    params.clone(),
                )
                .await?;
            if matches.len() != 1 {
                return Err(conflict());
            }
            let row = matches.into_iter().next().expect("exactly one checked row");
            let columns = row.columns_ref().to_vec();
            if row_values(row, &columns)? != *original {
                return Err(conflict());
            }
            if change.kind == "delete" {
                tx.exec_drop(
                    format!("DELETE FROM {table} WHERE {predicate} LIMIT 1"),
                    params,
                )
                .await?;
                if tx.affected_rows() != 1 {
                    return Err(conflict());
                }
                affected += 1;
            } else if let Some((sql, update_params)) =
                update_query(&info, &table, change, &predicate, params)?
            {
                tx.exec_drop(sql, update_params).await?;
                // A type coercion can produce the old value; the row was locked
                // and verified, so affected_rows=0 is a successful no-op.
                affected += tx.affected_rows();
            }
        }
        Ok(affected)
    }
    .await;
    match operation {
        Ok(affected) => {
            tx.commit().await?;
            Ok(affected)
        }
        Err(error) => {
            if let Err(rollback) = tx.rollback().await {
                return Err(AppError::new(
                    "ROLLBACK_FAILED",
                    format!(
                        "{}；回滚连接已中断，请刷新后核实数据：{rollback}",
                        error.message
                    ),
                ));
            }
            Err(error)
        }
    }
}

fn conflict() -> AppError {
    AppError::new(
        "EDIT_CONFLICT",
        "此行已被其他会话修改或删除。整批变更已回滚，请刷新后重试。",
    )
}

fn generated(field: &FieldInfo) -> bool {
    let extra = field.extra.to_ascii_uppercase();
    extra.contains("VIRTUAL GENERATED") || extra.contains("STORED GENERATED")
}

fn validate_change(info: &TableInfo, change: &RowChange) -> Result<()> {
    if !matches!(change.kind.as_str(), "insert" | "update" | "delete")
        || change.values.len() != info.fields.len()
    {
        return Err(AppError::new(
            "INVALID_CHANGE",
            "变更类型或字段数量无效，请刷新表结构",
        ));
    }
    if change.kind != "insert"
        && change
            .original
            .as_ref()
            .is_none_or(|v| v.len() != info.fields.len())
    {
        return Err(AppError::new(
            "INVALID_CHANGE",
            "原始行字段数量无效，请刷新表结构",
        ));
    }
    if change.kind == "update" {
        let original = change.original.as_ref().expect("validated original");
        for (i, f) in info.fields.iter().enumerate() {
            if generated(f)
                && change.values[i]
                    .as_ref()
                    .is_some_and(|value| value != &original[i])
            {
                return Err(AppError::new(
                    "READ_ONLY_COLUMN",
                    format!("生成字段 {} 不能修改", f.name),
                ));
            }
        }
    }
    Ok(())
}

fn key_predicate(info: &TableInfo, original: &[CellValue]) -> Result<(String, Vec<Value>)> {
    let mut predicates = Vec::new();
    let mut params = Vec::new();
    for key in &info.key_columns {
        let index = info
            .fields
            .iter()
            .position(|f| &f.name == key)
            .ok_or_else(|| AppError::new("METADATA", "唯一键字段不存在"))?;
        let value = original
            .get(index)
            .ok_or_else(|| AppError::new("INVALID_CHANGE", "原始行不完整"))?;
        if *value == CellValue::Null {
            return Err(AppError::new("INVALID_CHANGE", "唯一键不能为空"));
        }
        predicates.push(format!("{} = ?", quote_identifier(key)?));
        params.push(cell_param(value)?);
    }
    if predicates.is_empty() {
        return Err(AppError::new("READ_ONLY", "此表没有可用的唯一键"));
    }
    Ok((predicates.join(" AND "), params))
}

fn insert_query(info: &TableInfo, table: &str, change: &RowChange) -> Result<(String, Vec<Value>)> {
    let mut fields = Vec::new();
    let mut params = Vec::new();
    for (index, f) in info.fields.iter().enumerate() {
        if generated(f) {
            continue;
        }
        if let Some(value) = &change.values[index] {
            fields.push(quote_identifier(&f.name)?);
            params.push(cell_param(value)?);
        }
    }
    let marks = vec!["?"; params.len()].join(",");
    Ok((
        format!(
            "INSERT INTO {table} ({}) VALUES ({marks})",
            fields.join(",")
        ),
        params,
    ))
}

fn update_query(
    info: &TableInfo,
    table: &str,
    change: &RowChange,
    predicate: &str,
    key_params: Vec<Value>,
) -> Result<Option<(String, Vec<Value>)>> {
    let original = change
        .original
        .as_ref()
        .ok_or_else(|| AppError::new("INVALID_CHANGE", "缺少原始行"))?;
    let mut assignments = Vec::new();
    let mut params = Vec::new();
    for (index, f) in info.fields.iter().enumerate() {
        if generated(f) {
            continue;
        }
        if let Some(value) = &change.values[index] {
            if value != &original[index] {
                assignments.push(format!("{} = ?", quote_identifier(&f.name)?));
                params.push(cell_param(value)?);
            }
        }
    }
    if assignments.is_empty() {
        return Ok(None);
    }
    params.extend(key_params);
    Ok(Some((
        format!(
            "UPDATE {table} SET {} WHERE {predicate} LIMIT 1",
            assignments.join(",")
        ),
        params,
    )))
}

fn is_integer(kind: ColumnType) -> bool {
    matches!(
        kind,
        ColumnType::MYSQL_TYPE_TINY
            | ColumnType::MYSQL_TYPE_SHORT
            | ColumnType::MYSQL_TYPE_LONG
            | ColumnType::MYSQL_TYPE_LONGLONG
            | ColumnType::MYSQL_TYPE_INT24
            | ColumnType::MYSQL_TYPE_YEAR
    )
}
fn is_date(kind: ColumnType) -> bool {
    matches!(
        kind,
        ColumnType::MYSQL_TYPE_DATE
            | ColumnType::MYSQL_TYPE_NEWDATE
            | ColumnType::MYSQL_TYPE_TIME
            | ColumnType::MYSQL_TYPE_DATETIME
            | ColumnType::MYSQL_TYPE_TIMESTAMP
            | ColumnType::MYSQL_TYPE_TIMESTAMP2
            | ColumnType::MYSQL_TYPE_DATETIME2
            | ColumnType::MYSQL_TYPE_TIME2
    )
}
fn is_binary(column: &Column) -> bool {
    let kind = column.column_type();
    matches!(
        kind,
        ColumnType::MYSQL_TYPE_BIT
            | ColumnType::MYSQL_TYPE_GEOMETRY
            | ColumnType::MYSQL_TYPE_VECTOR
    ) || (column.character_set() == 63
        && !kind.is_numeric_type()
        && !is_date(kind)
        && !matches!(
            kind,
            ColumnType::MYSQL_TYPE_JSON | ColumnType::MYSQL_TYPE_NULL
        ))
}

pub fn column_meta(column: &Column) -> ColumnMeta {
    ColumnMeta {
        name: column.name_str().into_owned(),
        data_type: format!("{:?}", column.column_type())
            .trim_start_matches("MYSQL_TYPE_")
            .to_ascii_lowercase(),
        nullable: !column.flags().contains(ColumnFlags::NOT_NULL_FLAG),
        binary: is_binary(column),
    }
}

pub fn row_values(row: Row, columns: &[Column]) -> Result<Vec<CellValue>> {
    if row.len() != columns.len() {
        return Err(AppError::new("ROW_DECODE", "结果字段数量不一致"));
    }
    row.unwrap_raw()
        .into_iter()
        .zip(columns)
        .map(|(value, column)| {
            value
                .ok_or_else(|| AppError::new("ROW_DECODE", "结果字段已被读取"))
                .and_then(|value| decode_value(value, column))
        })
        .collect()
}

fn decode_value(value: Value, column: &Column) -> Result<CellValue> {
    let kind = column.column_type();
    let text = match value {
        Value::NULL => return Ok(CellValue::Null),
        Value::Bytes(bytes) => {
            if is_binary(column) {
                return Ok(CellValue::Binary(STANDARD.encode(bytes)));
            }
            String::from_utf8(bytes).map_err(|_| {
                AppError::new(
                    "ROW_DECODE",
                    "文本字段不是有效 UTF-8，请以二进制方式读取该字段",
                )
            })?
        }
        Value::Int(value) => value.to_string(),
        Value::UInt(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Double(value) => value.to_string(),
        Value::Date(year, month, day, hour, minute, second, micros) => {
            let date = format!("{year:04}-{month:02}-{day:02}");
            if matches!(
                kind,
                ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE
            ) {
                date
            } else {
                format!(
                    "{date} {hour:02}:{minute:02}:{second:02}{}",
                    fraction(micros, column.decimals())
                )
            }
        }
        Value::Time(negative, days, hours, minutes, seconds, micros) => {
            let hours = u64::from(days) * 24 + u64::from(hours);
            format!(
                "{}{hours:02}:{minutes:02}:{seconds:02}{}",
                if negative { "-" } else { "" },
                fraction(micros, column.decimals())
            )
        }
    };
    Ok(if is_integer(kind) {
        CellValue::Integer(text)
    } else if matches!(
        kind,
        ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL
    ) {
        CellValue::Decimal(text)
    } else if matches!(
        kind,
        ColumnType::MYSQL_TYPE_FLOAT | ColumnType::MYSQL_TYPE_DOUBLE
    ) {
        CellValue::Float(text)
    } else if is_date(kind) {
        CellValue::Date(text)
    } else {
        CellValue::Text(text)
    })
}

fn fraction(micros: u32, precision: u8) -> String {
    let precision = precision.min(6);
    if precision == 0 {
        String::new()
    } else {
        format!(".{}", &format!("{micros:06}")[..usize::from(precision)])
    }
}

pub fn cell_param(value: &CellValue) -> Result<Value> {
    match value {
        CellValue::Null => Ok(Value::NULL),
        CellValue::Text(v) | CellValue::Date(v) => Ok(Value::Bytes(v.as_bytes().to_vec())),
        CellValue::Binary(v) => STANDARD
            .decode(v)
            .map(Value::Bytes)
            .map_err(|_| AppError::new("INVALID_VALUE", "二进制值必须是有效的 base64")),
        CellValue::Integer(v) => {
            if let Ok(value) = v.parse::<i64>() {
                Ok(Value::Int(value))
            } else {
                v.parse::<u64>()
                    .map(Value::UInt)
                    .map_err(|_| AppError::new("INVALID_VALUE", "整数超出 MySQL BIGINT 范围"))
            }
        }
        CellValue::Decimal(v) => {
            let unsigned = v
                .strip_prefix('-')
                .or_else(|| v.strip_prefix('+'))
                .unwrap_or(v);
            let mut dots = 0;
            if unsigned.is_empty()
                || !unsigned.bytes().any(|c| c.is_ascii_digit())
                || !unsigned.bytes().all(|c| {
                    if c == b'.' {
                        dots += 1;
                        dots <= 1
                    } else {
                        c.is_ascii_digit()
                    }
                })
            {
                return Err(AppError::new("INVALID_VALUE", "DECIMAL 必须是十进制数值"));
            }
            Ok(Value::Bytes(v.as_bytes().to_vec()))
        }
        CellValue::Float(v) => {
            let parsed = v
                .parse::<f64>()
                .map_err(|_| AppError::new("INVALID_VALUE", "浮点数格式无效"))?;
            if !parsed.is_finite() {
                return Err(AppError::new("INVALID_VALUE", "MySQL 不支持 NaN 或无穷大"));
            }
            Ok(Value::Double(parsed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> TableInfo {
        TableInfo {
            fields: vec![
                FieldInfo {
                    name: "id".into(),
                    data_type: "bigint unsigned".into(),
                    nullable: false,
                    default_value: None,
                    extra: "auto_increment".into(),
                    comment: "".into(),
                },
                FieldInfo {
                    name: "na`me".into(),
                    data_type: "varchar(80)".into(),
                    nullable: true,
                    default_value: None,
                    extra: "".into(),
                    comment: "".into(),
                },
            ],
            indexes: vec![],
            foreign_keys: vec![],
            create_sql: String::new(),
            engine: Some("InnoDB".into()),
            key_columns: vec!["id".into()],
            editable: true,
        }
    }
    fn request() -> PageRequest {
        PageRequest {
            connection_id: "test".into(),
            database: "a`b".into(),
            table: "t".into(),
            page: 2,
            page_size: 200,
            sort_column: None,
            sort_desc: false,
            filters: vec![],
        }
    }

    #[test]
    fn identifiers_are_quoted_and_invalid_identifiers_rejected() {
        assert_eq!(
            quote_identifier("a`;DROP DATABASE db").unwrap(),
            "`a``;DROP DATABASE db`"
        );
        assert!(quote_identifier("").is_err());
        assert!(quote_identifier("a\0b").is_err());
        assert!(quote_identifier(&"x".repeat(65)).is_err());
        assert!(quote_identifier(&"中".repeat(64)).is_ok());
    }

    #[test]
    fn pagination_has_stable_order_and_bounded_offset() {
        let mut req = request();
        let (sql, params) = table_query(&info(), &req, true).unwrap();
        assert_eq!(
            sql,
            "SELECT `id`,`na``me` FROM `a``b`.`t` ORDER BY `id` ASC LIMIT ? OFFSET ?"
        );
        assert_eq!(params, vec![Value::UInt(201), Value::UInt(400)]);
        req.sort_column = Some("na`me".into());
        req.sort_desc = true;
        assert!(table_query(&info(), &req, true)
            .unwrap()
            .0
            .contains("ORDER BY `na``me` DESC,`id` ASC"));
        req.page_size = 5001;
        assert!(table_query(&info(), &req, true).is_err());
        req.page_size = 200;
        req.page = u64::MAX;
        assert!(table_query(&info(), &req, true).is_err());
    }

    #[test]
    fn filters_are_bound_and_contains_escapes_literal_wildcards() {
        let mut req = request();
        req.filters.push(Filter {
            column: "na`me".into(),
            operator: "contains".into(),
            value: Some(CellValue::Text("50%_!';drop table t".into())),
        });
        let (sql, params) = table_query(&info(), &req, false).unwrap();
        assert!(sql.contains("`na``me` LIKE ? ESCAPE '!'"));
        assert!(!sql.contains("drop table"));
        assert_eq!(
            params,
            vec![Value::Bytes(b"%50!%!_!!';drop table t%".to_vec())]
        );
        req.filters[0].operator = "= ? OR 1=1".into();
        assert!(table_query(&info(), &req, false).is_err());
        req.filters[0].operator = "eq".into();
        req.filters[0].column = "missing".into();
        assert!(table_query(&info(), &req, false).is_err());
    }

    #[test]
    fn null_filters_have_correct_mysql_semantics() {
        let mut req = request();
        req.filters.push(Filter {
            column: "na`me".into(),
            operator: "eq".into(),
            value: Some(CellValue::Null),
        });
        let (sql, params) = table_query(&info(), &req, false).unwrap();
        assert!(sql.contains("`na``me` IS NULL"));
        assert!(params.is_empty());
        req.filters[0].operator = "gt".into();
        assert!(table_query(&info(), &req, false).is_err());
    }

    #[test]
    fn numeric_and_binary_values_preserve_precision() {
        assert_eq!(
            cell_param(&CellValue::Integer("18446744073709551615".into())).unwrap(),
            Value::UInt(u64::MAX)
        );
        let decimal = "123456789012345678901234567890.000001";
        assert_eq!(
            cell_param(&CellValue::Decimal(decimal.into())).unwrap(),
            Value::Bytes(decimal.as_bytes().to_vec())
        );
        assert_eq!(
            cell_param(&CellValue::Binary("AP+A".into())).unwrap(),
            Value::Bytes(vec![0, 255, 128])
        );
        assert!(cell_param(&CellValue::Float("NaN".into())).is_err());
        assert!(cell_param(&CellValue::Decimal("1e10".into())).is_err());
        assert!(cell_param(&CellValue::Binary("!".into())).is_err());
    }

    #[test]
    fn text_protocol_and_binary_protocol_have_same_cell_representation() {
        let integer = Column::new(ColumnType::MYSQL_TYPE_LONGLONG).with_character_set(63);
        assert_eq!(
            decode_value(Value::Bytes(b"18446744073709551615".to_vec()), &integer).unwrap(),
            CellValue::Integer(u64::MAX.to_string())
        );
        assert_eq!(
            decode_value(Value::UInt(u64::MAX), &integer).unwrap(),
            CellValue::Integer(u64::MAX.to_string())
        );
        let timestamp = Column::new(ColumnType::MYSQL_TYPE_TIMESTAMP)
            .with_character_set(63)
            .with_decimals(3);
        assert_eq!(
            decode_value(Value::Date(2026, 10, 10, 2, 3, 4, 120_000), &timestamp).unwrap(),
            CellValue::Date("2026-10-10 02:03:04.120".into())
        );
        let time = Column::new(ColumnType::MYSQL_TYPE_TIME).with_decimals(6);
        assert_eq!(
            decode_value(Value::Time(true, 2, 3, 4, 5, 12), &time).unwrap(),
            CellValue::Date("-51:04:05.000012".into())
        );
        let json = Column::new(ColumnType::MYSQL_TYPE_JSON).with_character_set(63);
        assert_eq!(
            decode_value(Value::Bytes("{\"中\":1}".as_bytes().to_vec()), &json).unwrap(),
            CellValue::Text("{\"中\":1}".into())
        );
        let binary = Column::new(ColumnType::MYSQL_TYPE_BLOB).with_character_set(63);
        assert_eq!(
            decode_value(Value::Bytes(vec![0, 255]), &binary).unwrap(),
            CellValue::Binary("AP8=".into())
        );
    }

    #[test]
    fn updates_only_bind_changes_and_keep_old_key() {
        let change = RowChange {
            kind: "update".into(),
            original: Some(vec![
                CellValue::Integer("1".into()),
                CellValue::Text("old".into()),
            ]),
            values: vec![
                Some(CellValue::Integer("2".into())),
                Some(CellValue::Text("new".into())),
            ],
        };
        validate_change(&info(), &change).unwrap();
        let (predicate, keys) = key_predicate(&info(), change.original.as_ref().unwrap()).unwrap();
        let (sql, params) = update_query(&info(), "`d`.`t`", &change, &predicate, keys)
            .unwrap()
            .unwrap();
        assert_eq!(
            sql,
            "UPDATE `d`.`t` SET `id` = ?,`na``me` = ? WHERE `id` = ? LIMIT 1"
        );
        assert_eq!(
            params,
            vec![Value::Int(2), Value::Bytes(b"new".to_vec()), Value::Int(1)]
        );
    }

    #[test]
    fn defaults_and_generated_columns_are_omitted_on_insert() {
        let mut metadata = info();
        metadata.fields[1].extra = "STORED GENERATED".into();
        let change = RowChange {
            kind: "insert".into(),
            original: None,
            values: vec![None, Some(CellValue::Text("must omit".into()))],
        };
        validate_change(&metadata, &change).unwrap();
        assert_eq!(
            insert_query(&metadata, "`d`.`t`", &change).unwrap(),
            ("INSERT INTO `d`.`t` () VALUES ()".into(), vec![])
        );
        let change = RowChange {
            kind: "update".into(),
            original: Some(vec![
                CellValue::Integer("1".into()),
                CellValue::Text("old".into()),
            ]),
            values: vec![None, Some(CellValue::Text("new".into()))],
        };
        assert_eq!(
            validate_change(&metadata, &change).unwrap_err().code,
            "READ_ONLY_COLUMN"
        );
    }

    #[test]
    fn connection_options_enable_verified_tls_and_small_idle_pool() {
        let profile = ConnectionProfile {
            id: "1".into(),
            name: "Local".into(),
            host: "localhost".into(),
            port: 3306,
            username: "root".into(),
            database: None,
            tls: true,
            ca_pem: None,
            ca_name: None,
            remember_password: false,
        };
        let opts = options(&profile, "pw", None).unwrap();
        assert!(!opts.prefer_socket());
        assert_eq!(opts.stmt_cache_size(), 16);
        assert_eq!(
            opts.pool_opts().constraints(),
            PoolConstraints::new(0, 2).unwrap()
        );
        assert_eq!(
            opts.pool_opts().inactive_connection_ttl(),
            Duration::from_secs(60)
        );
        let ssl = opts.ssl_opts().unwrap();
        assert!(!ssl.skip_domain_validation());
        assert!(!ssl.accept_invalid_certs());
    }
}
