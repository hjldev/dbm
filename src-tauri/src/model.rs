use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub sql_state: Option<String>,
    pub statement_index: Option<usize>,
    pub line: Option<usize>,
}
pub type Result<T> = std::result::Result<T, AppError>;
impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            sql_state: None,
            statement_index: None,
            line: None,
        }
    }
}
impl From<mysql_async::Error> for AppError {
    fn from(error: mysql_async::Error) -> Self {
        if let mysql_async::Error::Server(server) = &error {
            Self {
                code: format!("MYSQL_{}", server.code),
                message: server.message.clone(),
                sql_state: Some(server.state.clone()),
                statement_index: None,
                line: None,
            }
        } else {
            Self::new("CONNECTION", error.to_string())
        }
    }
}
impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::new("IO", error.to_string())
    }
}
impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("CONFIG", error.to_string())
    }
}
impl From<csv::Error> for AppError {
    fn from(error: csv::Error) -> Self {
        Self::new("CSV", error.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub database: Option<String>,
    pub tls: bool,
    pub ca_pem: Option<String>,
    pub ca_name: Option<String>,
    #[serde(default)]
    pub remember_password: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    pub profile: ConnectionProfile,
    pub password: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    pub preview_limit: usize,
    pub sidebar_width: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            preview_limit: 1000,
            sidebar_width: 250,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentData {
    pub connections: Vec<ConnectionProfile>,
    pub settings: Settings,
    #[serde(default)]
    pub passwords: std::collections::HashMap<String, String>,
}

// Only this DTO crosses IPC; persisted credentials stay in the Rust backend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapData {
    pub connections: Vec<ConnectionProfile>,
    pub settings: Settings,
    pub config_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CellValue {
    Null,
    Text(String),
    Integer(String),
    Decimal(String),
    Float(String),
    Date(String),
    Binary(String),
}
impl CellValue {
    pub fn display(&self) -> String {
        match self {
            Self::Null => "\\N".into(),
            Self::Binary(v) => format!("base64:{v}"),
            Self::Text(v)
            | Self::Integer(v)
            | Self::Decimal(v)
            | Self::Float(v)
            | Self::Date(v) => v.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub binary: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub total_rows: u64,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementOutcome {
    pub index: usize,
    pub sql: String,
    pub line: usize,
    pub elapsed_ms: u64,
    pub affected_rows: String,
    pub last_insert_id: Option<String>,
    pub warnings: u16,
    pub results: Vec<ResultSet>,
    pub error: Option<AppError>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub execution_id: String,
    pub completed: usize,
    pub cancelled: bool,
    pub session_valid: bool,
    pub in_transaction: bool,
    pub database: Option<String>,
    pub error: Option<AppError>,
}
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ExecutionEvent {
    Statement {
        outcome: StatementOutcome,
    },
    Progress {
        index: usize,
        line: usize,
        sql: String,
    },
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectInfo {
    pub name: String,
    pub kind: String,
    pub engine: Option<String>,
    pub estimated_rows: Option<String>,
    pub comment: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub extra: String,
    pub comment: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub unique: bool,
    pub columns: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyInfo {
    pub name: String,
    pub column: String,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_column: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub fields: Vec<FieldInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub create_sql: String,
    pub engine: Option<String>,
    pub key_columns: Vec<String>,
    pub editable: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: String,
    pub operator: String,
    pub value: Option<CellValue>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub connection_id: String,
    pub database: String,
    pub table: String,
    pub page: u64,
    pub page_size: u32,
    pub sort_column: Option<String>,
    pub sort_desc: bool,
    pub filters: Vec<Filter>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult {
    pub result: ResultSet,
    pub has_next: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowChange {
    pub kind: String,
    pub original: Option<Vec<CellValue>>,
    pub values: Vec<Option<CellValue>>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequest {
    pub connection_id: String,
    pub database: String,
    pub table: String,
    pub changes: Vec<RowChange>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub execution_id: String,
    pub rows: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub name: String,
    pub content: String,
}
