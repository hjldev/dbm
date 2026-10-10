import { describe, expect, it } from 'vitest'
import { cellText, editedCell, editableField, equalCell, newRow, rowChange, validCell } from '../src/data'
import type { CellValue, FieldInfo } from '../src/types'
const text = (value: string): CellValue => ({ kind: 'text', value })
const field = (dataType = 'varchar(255)', extra = ''): FieldInfo => ({ name: 'value', dataType, nullable: true, defaultValue: null, extra, comment: '' })

describe('cell wire values', () => {
  it('preserves unsigned bigint and decimal precision as strings', () => {
    expect(editedCell('18446744073709551615', field('bigint unsigned'))).toEqual({ kind: 'integer', value: '18446744073709551615' })
    expect(editedCell('9007199254740993.123456789', field('decimal(40,9)'))).toEqual({ kind: 'decimal', value: '9007199254740993.123456789' })
  })
  it('distinguishes SQL NULL, default omission and literal NULL text', () => {
    expect(cellText(null)).toBe('DEFAULT')
    expect(cellText({ kind: 'null' })).toBe('NULL')
    expect(equalCell(text('NULL'), { kind: 'null' })).toBe(false)
    expect(editedCell('NULL', field())).toEqual(text('NULL'))
  })
  it('uses base64 for binary fields and rejects malformed values', () => {
    expect(editedCell('base64:AP8=', field('blob'))).toEqual({ kind: 'binary', value: 'AP8=' })
    expect(validCell({ kind: 'binary', value: 'AP8=' })).toBeNull()
    expect(validCell({ kind: 'binary', value: 'not a binary' })).toBeTruthy()
    expect(validCell({ kind: 'integer', value: '1 OR 1=1' })).toBeTruthy()
    expect(validCell({ kind: 'decimal', value: '1.20' })).toBeNull()
  })
})

describe('batch row edits', () => {
  const original = [{ kind: 'integer', value: '1' }, text('before'), { kind: 'null' }] as CellValue[]
  it('coalesces multiple edits into one update with unchanged columns omitted', () => {
    const edited = [...original]; edited[1] = text('中文,\n"quoted"')
    expect(rowChange(original, edited, false)).toEqual({ kind: 'update', original, values: [null, text('中文,\n"quoted"'), null] })
    expect(rowChange(original, [...original], false)).toBeNull()
  })
  it('submits explicit NULL but does not confuse it with an omitted value', () => {
    expect(rowChange(original, [original[0]!, { kind: 'null' }, original[2]!], false)?.values).toEqual([null, { kind: 'null' }, null])
  })
  it('update then delete yields just deletion using the original row', () => {
    expect(rowChange(original, [original[0]!, text('after'), original[2]!], true)).toEqual({ kind: 'delete', original, values: [null, null, null] })
  })
  it('insert then delete yields no backend change, defaults stay omitted', () => {
    expect(newRow([field('int', 'auto_increment'), field('varchar(20)'), field('int', 'VIRTUAL GENERATED')])).toEqual([null, null, null])
    expect(rowChange(null, [null, text('new')], false)).toEqual({ kind: 'insert', original: null, values: [null, text('new')] })
    expect(rowChange(null, [null, text('new')], true)).toBeNull()
    expect(editableField(field('int', 'STORED GENERATED'))).toBe(false)
    expect(editableField(field('int', 'auto_increment'))).toBe(true)
  })
})
