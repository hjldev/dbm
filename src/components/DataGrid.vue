<script setup lang="ts">
import { computed, ref, shallowRef } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { cellText, editedCell, editableField, validCell } from '../data'
import type { CellValue, ColumnMeta, FieldInfo } from '../types'
const props = withDefaults(defineProps<{ columns: ColumnMeta[]; rows: (CellValue | null)[][]; editable?: boolean; fields?: FieldInfo[]; selected?: Set<number>; states?: Record<number, string>; sortColumn?: string | null; sortDesc?: boolean }>(), { editable: false, fields: () => [], selected: () => new Set(), states: () => ({}) })
const emit = defineEmits<{ edit: [row: number, column: number, value: CellValue | null]; select: [row: number, value: boolean]; sort: [column: string]; selectAll: [value: boolean] }>()
const viewport = ref<HTMLElement | null>(null)
const virtualizer = useVirtualizer(computed(() => ({ count: props.rows.length, getScrollElement: () => viewport.value, estimateSize: () => 29, overscan: 15 })))
const virtualRows = computed(() => virtualizer.value.getVirtualItems())
const gridTemplate = computed(() => `48px ${props.columns.map(column => `minmax(${column.dataType.length > 25 ? 190 : 150}px, 1fr)`).join(' ')}`)
const width = computed(() => Math.max(48 + props.columns.length * 150, 200))
const editing = shallowRef<{ row: number; column: number; field: FieldInfo | ColumnMeta; original: CellValue | null } | null>(null)
const editText = ref('')
const editKind = ref<'value' | 'null' | 'default'>('value')
const error = ref('')
function openEditor(row: number, column: number) {
  if (!props.editable || props.states[row] === 'delete') return
  const field = props.fields[column] ?? props.columns[column]!
  if ('extra' in field && !editableField(field)) return
  const original = props.rows[row]![column] ?? null
  editing.value = { row, column, field, original }; editText.value = original && original.kind !== 'null' ? original.value : ''
  editKind.value = original === null ? 'default' : original.kind === 'null' ? 'null' : 'value'; error.value = ''
}
function applyEdit() {
  const entry = editing.value; if (!entry) return
  const value = editKind.value === 'null' ? { kind: 'null' as const } : editKind.value === 'default' ? null : editedCell(editText.value, entry.field)
  error.value = validCell(value) ?? ''
  if (error.value) return
  emit('edit', entry.row, entry.column, value); editing.value = null
}
function rowNumber(row: number) { return props.states[row] === 'insert' ? '+' : row + 1 }
</script>

<template>
  <div class="data-grid" role="table" :aria-rowcount="rows.length" :aria-colcount="columns.length">
    <div ref="viewport" class="grid-viewport">
      <div class="grid-content" :style="{ minWidth: `${width}px` }">
        <div class="grid-header grid-row" role="row" :style="{ gridTemplateColumns: gridTemplate }">
          <div class="grid-cell row-index"><input v-if="editable" type="checkbox" aria-label="选择当前页所有行" :checked="rows.length > 0 && selected.size === rows.length" @change="emit('selectAll', ($event.target as HTMLInputElement).checked)" /><span v-else>#</span></div>
          <button v-for="(column, index) in columns" :key="index" class="grid-cell column-heading" role="columnheader" :title="`${column.name} · ${column.dataType}`" @click="emit('sort', column.name)"><strong>{{ column.name }}</strong><span class="column-type">{{ column.dataType }}</span><span v-if="sortColumn === column.name" class="sort-arrow">{{ sortDesc ? '↓' : '↑' }}</span></button>
        </div>
        <div :style="{ height: `${virtualizer.getTotalSize()}px`, position: 'relative' }">
          <div v-for="item in virtualRows" :key="String(item.key)" class="grid-row data-row" :class="states[item.index]" role="row" :style="{ position: 'absolute', top: '0', transform: `translateY(${item.start}px)`, height: '29px', width: '100%', gridTemplateColumns: gridTemplate }">
            <div class="grid-cell row-index"><input v-if="editable" type="checkbox" :aria-label="`选择第 ${item.index + 1} 行`" :checked="selected.has(item.index)" @change="emit('select', item.index, ($event.target as HTMLInputElement).checked)" /><span v-else>{{ rowNumber(item.index) }}</span></div>
            <div v-for="(cell, column) in rows[item.index]" :key="column" class="grid-cell" :class="{ 'null-cell': cell?.kind === 'null', 'default-cell': cell === null, 'binary-cell': cell?.kind === 'binary' }" role="cell" :title="cellText(cell)" @dblclick="openEditor(item.index, column)">{{ cellText(cell) }}</div>
          </div>
        </div>
        <div v-if="!rows.length" class="grid-empty">{{ columns.length ? '没有符合条件的数据' : '运行查询后，结果将显示在这里' }}</div>
      </div>
    </div>
  </div>
  <Teleport to="body">
    <div v-if="editing" class="modal-backdrop" @mousedown.self="editing = null">
      <section class="modal cell-modal" role="dialog" aria-modal="true" aria-labelledby="cell-title">
        <header class="modal-header"><div><span class="eyebrow">EDIT VALUE · 第 {{ editing.row + 1 }} 行</span><h2 id="cell-title">{{ editing.field.name }} <span class="small muted">{{ editing.field.dataType }}</span></h2></div><button class="icon-button" aria-label="关闭" @click="editing = null">×</button></header>
        <form @submit.prevent="applyEdit"><div class="modal-body"><div class="segmented"><button type="button" :class="{ active: editKind === 'value' }" @click="editKind = 'value'">值</button><button type="button" :disabled="!editing.field.nullable" :class="{ active: editKind === 'null' }" @click="editKind = 'null'">NULL</button><button v-if="states[editing.row] === 'insert'" type="button" :class="{ active: editKind === 'default' }" @click="editKind = 'default'">默认值</button></div><textarea v-if="editKind === 'value'" v-model="editText" class="cell-input mono" autofocus rows="6" spellcheck="false"></textarea><p v-else class="helper">{{ editKind === 'null' ? '此字段将保存为 SQL NULL。' : '插入时省略此字段，由 MySQL 使用默认值或自动生成。' }}</p><p v-if="editing.field.dataType.match(/blob|binary|bit/i)" class="helper">二进制值使用 Base64 编码。</p><p v-if="error" class="text-danger">{{ error }}</p></div><footer class="modal-footer"><span class="muted small">变更仅在点击“保存变更”后提交</span><span class="spacer"></span><button type="button" class="button" @click="editing = null">取消</button><button type="submit" class="button primary">应用</button></footer></form>
      </section>
    </div>
  </Teleport>
</template>
