export interface AppError { code: string; message: string; sqlState?: string | null; statementIndex?: number | null; line?: number | null }
export interface ConnectionProfile { id: string; name: string; host: string; port: number; username: string; database: string | null; tls: boolean; caPem: string | null; caName: string | null; rememberPassword: boolean }
export interface Settings { theme: 'system' | 'light' | 'dark'; previewLimit: number; sidebarWidth: number }
export interface BootstrapData { connections: ConnectionProfile[]; settings: Settings; configPath: string }
export type CellValue = { kind: 'null' } | { kind: 'text' | 'integer' | 'decimal' | 'float' | 'date' | 'binary'; value: string }
export interface ColumnMeta { name: string; dataType: string; nullable: boolean; binary: boolean }
export interface ResultSet { columns: ColumnMeta[]; rows: CellValue[][]; totalRows: number; truncated: boolean }
export interface StatementOutcome { index: number; sql: string; line: number; elapsedMs: number; affectedRows: string; lastInsertId: string | null; warnings: number; results: ResultSet[]; error: AppError | null }
export interface ExecutionReport { executionId: string; completed: number; cancelled: boolean; sessionValid: boolean; inTransaction: boolean; database: string | null; error: AppError | null }
export type ExecutionEvent = { type: 'progress'; index: number; line: number } | { type: 'statement'; outcome: StatementOutcome }
export interface ObjectInfo { name: string; kind: 'table' | 'view'; engine: string | null; estimatedRows: string | null; comment: string }
export interface FieldInfo { name: string; dataType: string; nullable: boolean; defaultValue: string | null; extra: string; comment: string }
export interface IndexInfo { name: string; unique: boolean; columns: string[] }
export interface ForeignKeyInfo { name: string; column: string; referencedSchema: string; referencedTable: string; referencedColumn: string }
export interface TableInfo { fields: FieldInfo[]; indexes: IndexInfo[]; foreignKeys: ForeignKeyInfo[]; createSql: string; engine: string | null; keyColumns: string[]; editable: boolean }
export type SchemaAction = 'add' | 'modify' | 'drop'
export interface SchemaTarget { connectionId: string; database: string; table: string; kind: 'field' | 'index'; action: SchemaAction; originalName: string; expectedCreateSql: string }
export interface SchemaDefinition { name: string; definition: string; indexKind: 'primary' | 'unique' | 'index' | 'fulltext' | 'spatial' | null }
export interface SchemaChangeRequest { target: SchemaTarget; value: SchemaDefinition }
export interface Filter { column: string; operator: string; value: CellValue | null }
export interface PageRequest { connectionId: string; database: string; table: string; page: number; pageSize: number; sortColumn: string | null; sortDesc: boolean; filters: Filter[] }
export interface PageResult { result: ResultSet; hasNext: boolean }
export interface RowChange { kind: 'insert' | 'update' | 'delete'; original: CellValue[] | null; values: (CellValue | null)[] }
export interface ChangeRequest { connectionId: string; database: string; table: string; changes: RowChange[] }
export interface ExportProgress { executionId: string; rows: number }
export interface FileContent { name: string; content: string }
export interface SqlTab { id: string; kind: 'sql'; connectionId: string; database: string | null; title: string; source: string; savedSource: string; fileName: string | null; sessionId: string | null; inTransaction: boolean; sessionValid: boolean; busy: boolean; executionId: string | null; outcomes: StatementOutcome[]; outcomeOffset: number; selectedOutcome: number; selectedResult: number; progress: string; report: ExecutionReport | null }
export interface TableTab { id: string; kind: 'table'; connectionId: string; database: string; table: string; title: string; dirty: boolean; schemaDirty?: boolean; busy: boolean }
export type WorkspaceTab = SqlTab | TableTab
