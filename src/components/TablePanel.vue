<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from 'vue'
import { Channel, command } from '../api'
import { useDbm } from '../store'
import { editedCell, newRow, rowChange } from '../data'
import type { CellValue, ExportProgress, Filter, PageRequest, PageResult, RowChange, SchemaAction, SchemaTarget, TableInfo, TableTab } from '../types'
import DataGrid from './DataGrid.vue'
import SchemaDialog from './SchemaDialog.vue'
const props = defineProps<{ tab: TableTab }>()
const dbm = useDbm()
const info = shallowRef<TableInfo | null>(null)
const pageResult = shallowRef<PageResult | null>(null)
const entries = shallowRef<{ original: CellValue[] | null; values: (CellValue | null)[]; deleted: boolean }[]>([])
const selected = shallowRef(new Set<number>())
const section = ref('data')
const page = ref(0)
const pageSize = ref(200)
const sortColumn = ref<string | null>(null)
const sortDesc = ref(false)
const error = ref('')
const filterVisible = ref(false)
const filterDraft = ref<{ id: string; column: string; operator: string; text: string }[]>([])
const filters = ref<Filter[]>([])
const exportId = ref<string | null>(null)
const exportRows = ref(0)
const exportReady = ref(false)
const loading = ref(true)
const schemaTarget = ref<SchemaTarget | null>(null)
const operatorLabels: Record<string, string> = { eq: '等于', ne: '不等于', contains: '包含', gt: '大于', gte: '大于等于', lt: '小于', lte: '小于等于', isNull: '为 NULL', isNotNull: '非 NULL' }
const changes = computed(() => entries.value.map(entry => rowChange(entry.original, entry.values, entry.deleted)).filter((change): change is RowChange => change !== null))
const states = computed(() => Object.fromEntries(entries.value.flatMap((entry, index) => { const change = rowChange(entry.original, entry.values, entry.deleted); return change ? [[index, change.kind]] : [] })))
const rows = computed(() => entries.value.map(entry => entry.values))
const editable = computed(() => !!info.value?.editable && !props.tab.busy)
const schemaEditable = computed(() => !!info.value?.engine && !props.tab.busy && !props.tab.dirty)
function editSchema(kind: 'field' | 'index', originalName: string, action: SchemaAction = 'modify') {
  if (!schemaEditable.value || !info.value) return
  schemaTarget.value = { connectionId: props.tab.connectionId, database: props.tab.database, table: props.tab.table, kind, action, originalName, expectedCreateSql: info.value.createSql }
}
async function schemaSaved() {
  schemaTarget.value = null
  page.value = 0; sortColumn.value = null; sortDesc.value = false; filters.value = []; filterDraft.value = []
  entries.value = []; pageResult.value = null; info.value = null
  await load()
}
async function schemaRefresh() { schemaTarget.value = null; await refresh() }
const request = (): PageRequest => ({ connectionId: props.tab.connectionId, database: props.tab.database, table: props.tab.table, page: page.value, pageSize: pageSize.value, sortColumn: sortColumn.value, sortDesc: sortDesc.value, filters: filters.value })
watch(changes, value => { props.tab.dirty = value.length > 0 }, { flush: 'sync' })
async function discardAllowed() { return !props.tab.dirty || await dbm.ask('放弃未保存的数据变更？', '此操作会重新加载数据，当前待提交变更将被丢弃。', '放弃并继续', true) }
async function load() {
  loading.value = true; props.tab.busy = true; error.value = ''
  try {
    if (!info.value) info.value = await command<TableInfo>('describe_table', { connectionId: props.tab.connectionId, database: props.tab.database, table: props.tab.table })
    const result = await command<PageResult>('read_page', { request: request() })
    pageResult.value = result; entries.value = result.result.rows.map(row => ({ original: row, values: [...row], deleted: false })); selected.value = new Set()
  } catch (issue) { error.value = (issue as { message: string }).message; dbm.showError(issue) }
  finally { loading.value = false; props.tab.busy = false }
}
onMounted(load)
async function refresh() { if (props.tab.busy || !await discardAllowed()) return; info.value = null; await load() }
async function changePage(next: number) { if (props.tab.busy || !await discardAllowed()) return; page.value = next; await load() }
async function changeSize(event: Event) { if (props.tab.busy) return; const next = Number((event.target as HTMLSelectElement).value); if (!await discardAllowed()) { (event.target as HTMLSelectElement).value = String(pageSize.value); return }; pageSize.value = next; page.value = 0; await load() }
async function sort(column: string) { if (props.tab.busy || !await discardAllowed()) return; sortDesc.value = sortColumn.value === column ? !sortDesc.value : false; sortColumn.value = column; page.value = 0; await load() }
function addFilter() { filterDraft.value.push({ id: crypto.randomUUID(), column: info.value?.fields[0]?.name ?? '', operator: 'eq', text: '' }) }
async function applyFilters() {
  if (props.tab.busy || !await discardAllowed()) return
  filters.value = filterDraft.value.map(filter => ({ column: filter.column, operator: filter.operator, value: filter.operator === 'isNull' || filter.operator === 'isNotNull' ? null : editedCell(filter.text, info.value!.fields.find(field => field.name === filter.column)!) }))
  page.value = 0; await load()
}
async function clearFilters() { if (props.tab.busy || !await discardAllowed()) return; filters.value = []; filterDraft.value = []; page.value = 0; await load() }
function edit(row: number, column: number, value: CellValue | null) { entries.value = entries.value.map((entry, index) => index === row ? { ...entry, values: entry.values.map((cell, i) => i === column ? value : cell) } : entry) }
function select(row: number, value: boolean) { const next = new Set(selected.value); value ? next.add(row) : next.delete(row); selected.value = next }
function selectAll(value: boolean) { selected.value = value ? new Set(entries.value.map((_, index) => index)) : new Set() }
function insert() { if (!info.value || !editable.value) return; entries.value = [...entries.value, { original: null, values: newRow(info.value.fields), deleted: false }]; selected.value = new Set([entries.value.length - 1]) }
function remove() {
  if (!editable.value) return
  entries.value = entries.value.map((entry, index) => selected.value.has(index) ? { ...entry, deleted: true } : entry).filter(entry => entry.original || !entry.deleted)
  selected.value = new Set()
}
async function revert() { if (!changes.value.length || !await dbm.ask('撤销全部待提交变更？', '数据库中已保存的数据不会受到影响。', '撤销变更')) return; entries.value = entries.value.filter(entry => entry.original).map(entry => ({ original: entry.original, values: [...entry.original!], deleted: false })); selected.value = new Set() }
async function commit() {
  if (!changes.value.length || props.tab.busy) return
  if (changes.value.some(change => change.kind === 'delete') && !await dbm.ask('提交包含删除的变更？', `将提交 ${changes.value.length} 行变更，其中 ${changes.value.filter(change => change.kind === 'delete').length} 行将被删除。`, '保存变更', true)) return
  props.tab.busy = true
  try { const count = await command<number>('apply_changes', { request: { connectionId: props.tab.connectionId, database: props.tab.database, table: props.tab.table, changes: changes.value } }); dbm.notify(`已提交 ${count} 行变更`); props.tab.dirty = false; await load() }
  catch (issue) { dbm.showError(issue); props.tab.busy = false }
}
async function exportTable() {
  if (props.tab.busy) return
  if (props.tab.dirty && !await dbm.ask('导出数据库中已保存的数据？', '尚未提交的本地变更不会包含在导出的 CSV 中。', '导出已保存数据')) return
  props.tab.busy = true; exportId.value = crypto.randomUUID(); exportRows.value = 0; exportReady.value = false
  const events = new Channel<ExportProgress>(); events.onmessage = event => { exportRows.value = event.rows; exportReady.value = true }
  try { const path = await command<string | null>('export_table', { request: request(), executionId: exportId.value, events }); if (path) dbm.notify(`CSV 已导出：${path}`) }
  catch (issue) { dbm.showError(issue) }
  finally { props.tab.busy = false; exportId.value = null; exportReady.value = false }
}
</script>

<template>
  <section class="table-panel workspace-panel">
    <header class="workspace-heading"><div class="breadcrumbs"><span>{{ dbm.connections.find(profile => profile.id === tab.connectionId)?.name }}</span><span>/</span><span>{{ tab.database }}</span><span>/</span><strong>{{ tab.table }}</strong><span class="badge">{{ info?.engine || 'VIEW' }}</span></div><div class="toolbar-actions"><button class="button small" :disabled="tab.busy" @click="dbm.newQuery(tab.connectionId, tab.database)">＋ SQL 查询</button><button class="icon-button" title="刷新数据与结构" :disabled="tab.busy" @click="refresh">↻</button></div></header>
    <div class="section-tabs"><button v-for="item in [['data','数据'],['fields','字段'],['indexes','索引'],['foreignKeys','外键'],['ddl','建表 SQL']]" :key="item[0]" :class="{ active: section === item[0] }" @click="section = item[0]!">{{ item[1] }}</button><span class="spacer"></span><span v-if="changes.length" class="pending-badge">{{ changes.length }} 行待提交</span><span v-else class="muted small">{{ info?.fields.length || 0 }} 个字段</span></div>
    <template v-if="section === 'data'">
      <div class="table-toolbar"><div class="toolbar-actions"><button class="button small" :disabled="!editable" @click="insert">＋ 新增行</button><button class="button small" :disabled="!editable || !selected.size" @click="remove">− 删除行</button><span class="toolbar-divider"></span><button class="button small" :class="{ selected: filterVisible || filters.length }" :disabled="tab.busy" @click="filterVisible = !filterVisible; if (filterVisible && !filterDraft.length) addFilter()">筛选<span v-if="filters.length" class="count-dot">{{ filters.length }}</span></button><button class="button small" :disabled="tab.busy" @click="exportTable">导出 CSV</button></div><span class="spacer"></span><button class="button small ghost" :disabled="!changes.length || tab.busy" @click="revert">撤销</button><button class="button small primary" :disabled="!changes.length || tab.busy" @click="commit">保存变更{{ changes.length ? ` (${changes.length})` : '' }}</button></div>
      <div v-if="filterVisible" class="filter-panel"><div v-for="filter in filterDraft" :key="filter.id" class="filter-row"><span class="muted small">AND</span><select v-model="filter.column" aria-label="筛选字段"><option v-for="field in info?.fields" :key="field.name" :value="field.name">{{ field.name }}</option></select><select v-model="filter.operator" aria-label="筛选条件"><option v-for="(label, op) in operatorLabels" :key="op" :value="op">{{ label }}</option></select><input v-if="!['isNull','isNotNull'].includes(filter.operator)" v-model="filter.text" placeholder="输入筛选值" @keydown.enter="applyFilters" /><span v-else class="spacer"></span><button class="icon-button" aria-label="移除此筛选" @click="filterDraft = filterDraft.filter(item => item.id !== filter.id)">×</button></div><div class="toolbar-actions"><button class="button small ghost" :disabled="tab.busy" @click="addFilter">＋ 条件</button><span class="spacer"></span><button class="button small" :disabled="tab.busy" @click="clearFilters">清除</button><button class="button small primary" :disabled="tab.busy" @click="applyFilters">应用筛选</button></div></div>
      <div v-if="!info?.editable && info" class="notice-strip"><span>◎</span><span>只读浏览 · {{ info.engine?.toLowerCase() !== 'innodb' ? '图形编辑仅支持 InnoDB 表' : '此表没有主键或非空唯一键' }}，可使用 SQL 修改。</span></div>
      <div v-if="exportId" class="notice-strip"><span class="spinner"></span><span>{{ exportReady ? `正在流式导出 · 已处理 ${exportRows.toLocaleString()} 行` : '准备导出…' }}</span><button class="button small" :disabled="!exportReady" @click="dbm.cancel(exportId)">取消导出</button></div>
      <div class="grid-region"><DataGrid v-if="pageResult" :columns="pageResult.result.columns" :rows="rows" :fields="info?.fields" :editable="editable" :selected="selected" :states="states" :sort-column="sortColumn" :sort-desc="sortDesc" @edit="edit" @select="select" @select-all="selectAll" @sort="sort" /><div v-if="loading" class="loading-overlay"><span class="spinner"></span>正在加载数据…</div><div v-else-if="error && !pageResult" class="empty-message"><b>无法加载数据</b><p>{{ error }}</p><button class="button" @click="refresh">重试</button></div></div>
      <footer class="result-footer"><span>{{ pageResult?.result.rows.length.toLocaleString() || 0 }} 行 · 第 {{ page + 1 }} 页</span><span v-if="editable" class="muted">双击单元格编辑</span><span class="spacer"></span><label class="page-size">每页<select :value="pageSize" :disabled="tab.busy" aria-label="每页行数" @change="changeSize"><option :value="100">100</option><option :value="200">200</option><option :value="500">500</option><option :value="1000">1000</option></select></label><button class="button small" :disabled="!page || tab.busy" @click="changePage(page - 1)">‹ 上一页</button><button class="button small" :disabled="!pageResult?.hasNext || tab.busy" @click="changePage(page + 1)">下一页 ›</button></footer>
    </template>
    <div v-else class="metadata-panel">
      <template v-if="['fields', 'indexes'].includes(section) && info">
        <div class="table-toolbar"><button v-if="section === 'fields'" class="button small" :disabled="!schemaEditable" @click="editSchema('field', '', 'add')">＋ 新增字段</button><button v-else class="button small" :disabled="!schemaEditable" @click="editSchema('index', '', 'add')">＋ 新增索引</button><span class="spacer"></span><span class="muted small">{{ section === 'fields' ? `${info.fields.length} 个字段` : `${info.indexes.length} 个索引` }}</span></div>
        <div class="notice-strip"><span>{{ !info.engine ? '视图结构请通过 SQL 修改。' : tab.dirty ? '请先保存或撤销待提交的数据，再修改表结构。' : '支持新增、修改和删除，预览 SQL 后执行。' }}</span></div>
      </template>
      <table v-if="section === 'fields'" class="metadata-table"><thead><tr><th>字段名称</th><th>类型</th><th>可为空</th><th>默认值</th><th>额外属性</th><th>注释</th><th>操作</th></tr></thead><tbody><tr v-for="field in info?.fields" :key="field.name"><td class="mono"><span v-if="info?.keyColumns.includes(field.name)" class="key-mark">◆ </span>{{ field.name }}</td><td class="mono">{{ field.dataType }}</td><td>{{ field.nullable ? '是' : '否' }}</td><td class="mono muted">{{ field.defaultValue ?? '—' }}</td><td>{{ field.extra || '—' }}</td><td>{{ field.comment || '—' }}</td><td><div class="toolbar-actions"><button class="button small" :disabled="!schemaEditable" :aria-label="`修改字段 ${field.name}`" @click="editSchema('field', field.name)">修改</button><button class="button small delete-schema" :disabled="!schemaEditable" :aria-label="`删除字段 ${field.name}`" @click="editSchema('field', field.name, 'drop')">删除</button></div></td></tr></tbody></table>
      <table v-else-if="section === 'indexes'" class="metadata-table"><thead><tr><th>索引名称</th><th>类型</th><th>字段</th><th>操作</th></tr></thead><tbody><tr v-for="index in info?.indexes" :key="index.name"><td class="mono">{{ index.name }}</td><td><span class="badge">{{ index.name === 'PRIMARY' ? '主键' : index.unique ? '唯一索引' : '普通索引' }}</span></td><td class="mono">{{ index.columns.join(', ') || '表达式索引' }}</td><td><div class="toolbar-actions"><button class="button small" :disabled="!schemaEditable" :aria-label="`修改索引 ${index.name}`" @click="editSchema('index', index.name)">修改</button><button class="button small delete-schema" :disabled="!schemaEditable" :aria-label="`删除索引 ${index.name}`" @click="editSchema('index', index.name, 'drop')">删除</button></div></td></tr><tr v-if="info && !info.indexes.length"><td colspan="4" class="muted">此表没有索引</td></tr></tbody></table>
      <table v-else-if="section === 'foreignKeys'" class="metadata-table"><thead><tr><th>外键名称</th><th>字段</th><th>引用数据库 / 表</th><th>引用字段</th></tr></thead><tbody><tr v-for="key in info?.foreignKeys" :key="`${key.name}:${key.column}`"><td class="mono">{{ key.name }}</td><td class="mono">{{ key.column }}</td><td class="mono">{{ key.referencedSchema }}.{{ key.referencedTable }}</td><td class="mono">{{ key.referencedColumn }}</td></tr><tr v-if="!info?.foreignKeys.length"><td colspan="4" class="muted">此表没有外键</td></tr></tbody></table>
      <pre v-else class="ddl-view">{{ info?.createSql || '正在加载…' }}</pre>
    </div>
    <SchemaDialog v-if="schemaTarget" :target="schemaTarget" :tab="tab" @close="schemaTarget = null" @saved="schemaSaved" @refresh="schemaRefresh" />
  </section>
</template>
