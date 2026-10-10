import type { CellValue, ColumnMeta, FieldInfo, RowChange } from './types'

export function cellText(cell: CellValue | null): string {
  if (cell === null) return 'DEFAULT'
  return cell.kind === 'null' ? 'NULL' : cell.kind === 'binary' ? `base64:${cell.value}` : cell.value
}
export function equalCell(left: CellValue | null, right: CellValue | null): boolean { return JSON.stringify(left) === JSON.stringify(right) }
export function editedCell(text: string, field: FieldInfo | ColumnMeta): CellValue {
  if ('binary' in field && field.binary || /^(?:binary|varbinary|.*blob|bit)\b/i.test(field.dataType)) return { kind: 'binary', value: text.replace(/^base64:/, '') }
  if (/^(?:tinyint|smallint|mediumint|int|bigint)\b/i.test(field.dataType)) return { kind: 'integer', value: text }
  if (/^(?:decimal|numeric)\b/i.test(field.dataType)) return { kind: 'decimal', value: text }
  if (/^(?:float|double|real)\b/i.test(field.dataType)) return { kind: 'float', value: text }
  if (/^(?:date|datetime|timestamp|time|year)\b/i.test(field.dataType)) return { kind: 'date', value: text }
  return { kind: 'text', value: text }
}
export function editableField(field: FieldInfo): boolean { return !/\b(?:VIRTUAL|STORED) GENERATED\b/i.test(field.extra) }
export function newRow(fields: FieldInfo[]): (CellValue | null)[] { return fields.map(() => null) }
export function changedValues(original: CellValue[], values: (CellValue | null)[]): (CellValue | null)[] { return values.map((value, index) => equalCell(value, original[index] ?? null) ? null : value) }
export function rowChange(original: CellValue[] | null, values: (CellValue | null)[], deleted: boolean): RowChange | null {
  if (!original) return deleted ? null : { kind: 'insert', original: null, values }
  if (deleted) return { kind: 'delete', original, values: original.map(() => null) }
  const changed = changedValues(original, values)
  return changed.some(value => value !== null) ? { kind: 'update', original, values: changed } : null
}
export function validCell(cell: CellValue | null): string | null {
  if (!cell || cell.kind === 'null') return null
  if (cell.kind === 'integer' && !/^[+-]?\d+$/.test(cell.value)) return '整数格式不正确'
  if ((cell.kind === 'decimal' || cell.kind === 'float') && !/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(cell.value)) return '数值格式不正确'
  if (cell.kind === 'binary' && !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(cell.value)) return '二进制字段使用 Base64 格式'
  return null
}
