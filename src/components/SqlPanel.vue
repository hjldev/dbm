<script setup lang="ts">
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { command } from '../api'
import { MAX_OUTCOMES, useDbm } from '../store'
import type { FileContent, SqlTab } from '../types'
import DataGrid from './DataGrid.vue'
const SqlEditor = defineAsyncComponent(() => import('./SqlEditor.vue'))
const props = defineProps<{ tab: SqlTab }>()
const dbm = useDbm()
const editor = ref<{ selection: () => string; cursor: () => number; focus: () => void } | null>(null)
const schema = shallowRef<Record<string, string[]>>({})
const schemaLoaded = new Set<string>()
const resultView = ref<'result' | 'log'>('result')
const editorHeight = ref(43)
const preparing = ref(false)
const container = ref<HTMLElement | null>(null)
const outcome = computed(() => props.tab.outcomes[props.tab.selectedOutcome] ?? null)
const result = computed(() => outcome.value?.results[props.tab.selectedResult] ?? null)
const busy = computed(() => props.tab.busy || preparing.value)
const profile = computed(() => dbm.connections.find(item => item.id === props.tab.connectionId))
const currentDb = computed(() => props.tab.database ?? profile.value?.database ?? null)
watch(currentDb, () => { schema.value = {}; schemaLoaded.clear() })
watch(() => [dbm.objects, currentDb.value], () => {
  const next: Record<string, string[]> = { ...schema.value }
  for (const object of dbm.objects[`${props.tab.connectionId}:${currentDb.value}`] ?? []) next[object.name] ??= []
  schema.value = next
}, { immediate: true, deep: true })
let completionTimer: ReturnType<typeof setTimeout> | undefined
let unmounted = false
watch(() => props.tab.source, source => {
  clearTimeout(completionTimer)
  completionTimer = setTimeout(async () => {
    if (!currentDb.value) return
    const names = [...source.matchAll(/\b(?:FROM|JOIN|UPDATE|INTO)\s+(`[^`]+`|[\w$]+)/gi)].map(match => match[1]!.replace(/^`|`$/g, ''))
    for (const table of new Set(names)) {
      const key = `${currentDb.value}.${table}`
      if (schemaLoaded.has(key)) continue
      schemaLoaded.add(key)
      try { const database: string | null = currentDb.value; const columns = await command<string[]>('list_columns', { connectionId: props.tab.connectionId, database, table }); if (!unmounted && database === currentDb.value) schema.value = { ...schema.value, [table]: columns } }
      catch { /* incomplete SQL and unknown tables should not interrupt editing */ }
    }
  }, 350)
})
onMounted(async () => { if (currentDb.value && !dbm.objects[`${props.tab.connectionId}:${currentDb.value}`]) await dbm.expandDatabase(props.tab.connectionId, currentDb.value, true) })
onBeforeUnmount(() => { unmounted = true; clearTimeout(completionTimer) })
async function run(mode: 'current' | 'all' | 'selection') {
  if (busy.value || !props.tab.sessionValid) return
  let source = props.tab.source
  const selection = editor.value?.selection() ?? ''
  if (mode !== 'all' && selection.trim()) source = selection
  else if (mode === 'selection') { dbm.notify('请先选中要执行的 SQL', true); return }
  else if (mode === 'current') {
    preparing.value = true; props.tab.busy = true
    try { const sessionId = await dbm.ensureSession(props.tab); source = await command<string>('statement_at_cursor', { source, cursor: editor.value?.cursor() ?? 0, sessionId }) }
    catch (error) { dbm.showError(error); preparing.value = false; props.tab.busy = false; return }
    preparing.value = false; props.tab.busy = false
  }
  if (!source.trim()) { dbm.notify('请输入要执行的 SQL'); return }
  resultView.value = 'result'
  await dbm.execute(props.tab, source)
  if (outcome.value?.error && !outcome.value.results.length) resultView.value = 'log'
}
async function open() {
  if (props.tab.source !== props.tab.savedSource && !await dbm.ask('替换当前 SQL？', '当前尚未保存的 SQL 内容将被替换。', '打开文件', true)) return
  try { const file = await command<FileContent | null>('open_sql_file'); if (file) { props.tab.source = file.content; props.tab.savedSource = file.content; props.tab.fileName = file.name; props.tab.title = file.name } }
  catch (error) { dbm.showError(error) }
}
async function save() {
  try { const path = await command<string | null>('save_sql_file', { source: props.tab.source, name: props.tab.fileName ?? `${props.tab.title}.sql` }); if (path) { props.tab.savedSource = props.tab.source; props.tab.fileName = path.split('/').pop() ?? path; props.tab.title = props.tab.fileName; dbm.notify(`SQL 已保存：${path}`) } }
  catch (error) { dbm.showError(error) }
}
async function exportPreview() { if (!result.value) return; try { const path = await command<string | null>('export_preview', { result: result.value }); if (path) dbm.notify(`CSV 已导出：${path}`) } catch (error) { dbm.showError(error) } }
async function changeDatabase(event: Event) {
  const database = (event.target as HTMLSelectElement).value || null
  if (!props.tab.sessionId) props.tab.database = database
  else if (database) await dbm.execute(props.tab, `USE \`${database.replaceAll('`', '``')}\`;`)
  if (database) await dbm.expandDatabase(props.tab.connectionId, database, true)
}
function resize(event: MouseEvent) {
  event.preventDefault()
  const move = (next: MouseEvent) => { const rect = container.value!.getBoundingClientRect(); editorHeight.value = Math.min(72, Math.max(23, (next.clientY - rect.top) / rect.height * 100)) }
  const stop = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', stop) }
  window.addEventListener('mousemove', move); window.addEventListener('mouseup', stop)
}
function clearResults() { dbm.clearResults(props.tab) }
defineExpose({ save, open })
</script>

<template>
  <section class="sql-panel workspace-panel">
    <header class="workspace-heading"><div class="breadcrumbs"><span class="connection-dot"></span><strong>{{ profile?.name }}</strong><span>/</span><select :value="tab.database || ''" :disabled="busy" aria-label="当前 SQL 数据库" @change="changeDatabase"><option v-if="!tab.sessionId" value="">未选择数据库</option><option v-for="database in dbm.databases[tab.connectionId]" :key="database" :value="database">{{ database }}</option></select><span v-if="tab.inTransaction" class="transaction-badge">事务未提交</span><span v-if="!tab.sessionValid" class="text-danger small">会话失效</span></div><div class="toolbar-actions"><button class="button small ghost" :disabled="busy" @click="open">打开 SQL</button><button class="button small ghost" @click="save">保存 SQL</button></div></header>
    <div class="sql-toolbar"><button class="button small primary" :disabled="busy || !tab.sessionValid" title="执行当前语句或选中内容（⌘ Enter）" @click="run('current')"><span>▶</span> 执行<span class="shortcut">⌘ ↵</span></button><button class="button small" :disabled="busy || !tab.sessionValid" title="执行全部（⌘ Shift Enter）" @click="run('all')">执行全部</button><button class="button small ghost" :disabled="busy || !tab.sessionValid" @click="run('selection')">执行选中</button><span class="toolbar-divider"></span><button class="button small" :disabled="!tab.executionId" @click="dbm.cancel(tab.executionId)"><span class="text-danger">■</span> 停止</button><span class="spacer"></span><span class="muted small">预览上限</span><select :value="dbm.settings.previewLimit" :disabled="busy" aria-label="查询预览行数" @change="dbm.updateSettings({ previewLimit: Number(($event.target as HTMLSelectElement).value) })"><option v-for="size in [100, 1000, 5000, 10000]" :key="size" :value="size">{{ size.toLocaleString() }} 行</option></select></div>
    <div ref="container" class="query-content"><div class="editor-region" :style="{ height: `${editorHeight}%` }"><SqlEditor ref="editor" v-model="tab.source" :schema="schema" :database="tab.database" :disabled="busy" @run="run" /><div v-if="!tab.source" class="editor-placeholder"><span>-- 在这里编写 MySQL 查询</span><span>-- ⌘ Enter 执行当前语句 · ⌘ Shift Enter 执行全部</span></div></div><div class="resize-handle" @mousedown="resize"></div>
      <div class="query-results"><div class="section-tabs"><button :class="{ active: resultView === 'result' }" @click="resultView = 'result'">查询结果<span v-if="result" class="tab-counter">{{ result.rows.length }}</span></button><button :class="{ active: resultView === 'log' }" @click="resultView = 'log'">执行记录<span v-if="tab.outcomes.length" class="tab-counter">{{ tab.outcomeOffset + tab.outcomes.length }}</span></button><span class="spacer"></span><button class="button small ghost" :disabled="busy || !tab.outcomes.length" @click="clearResults">清空</button><button class="button small ghost" :disabled="!result?.rows.length" @click="exportPreview">导出预览 CSV</button></div>
        <template v-if="resultView === 'result'"><div v-if="tab.outcomes.length" class="result-selector"><select v-model.number="tab.selectedOutcome" aria-label="选择语句结果" @change="tab.selectedResult = 0"><option v-for="(item, index) in tab.outcomes" :key="tab.outcomeOffset + index" :value="index">{{ tab.outcomeOffset + index + 1 }} · {{ item.error ? '失败' : item.results.length ? '结果集' : `${item.affectedRows} 行受影响` }} · {{ item.sql.slice(0, 200).replace(/\s+/g, ' ').slice(0, 85) }}</option></select><button v-for="(_, index) in outcome?.results" :key="index" class="button small" :class="{ selected: tab.selectedResult === index }" @click="tab.selectedResult = index">结果 {{ index + 1 }}</button><span class="spacer"></span><span class="muted small">{{ outcome?.elapsedMs }} ms</span></div><div v-if="result?.truncated" class="notice-strip">◎ {{ result.rows.length ? `显示前 ${result.rows.length.toLocaleString()} 行，实际返回 ${result.totalRows.toLocaleString()} 行。` : '此结果预览已释放，仅保留执行摘要。' }} CSV 只导出保留的预览。</div><DataGrid v-if="result" :columns="result.columns" :rows="result.rows" /><div v-else class="query-empty"><div class="empty-glyph">⌘</div><h3>{{ outcome?.error ? '查询执行失败' : outcome ? '语句执行完成' : '准备好开始查询' }}</h3><p>{{ outcome?.error?.message || (outcome ? `${outcome.affectedRows} 行受影响${outcome.lastInsertId ? ` · 最后插入 ID：${outcome.lastInsertId}` : ''}` : '选择数据库，在上方编写 SQL 并执行。') }}</p></div></template>
        <div v-else class="execution-log"><div v-if="!tab.outcomes.length" class="empty-message muted">尚无执行记录</div><div v-if="tab.outcomeOffset" class="notice-strip">◎ 已省略较早的 {{ tab.outcomeOffset.toLocaleString() }} 条记录，执行记录仅保留最近 {{ MAX_OUTCOMES.toLocaleString() }} 条。</div><article v-for="(item, index) in tab.outcomes" :key="tab.outcomeOffset + index" class="log-entry" :class="{ failed: item.error }"><div class="log-heading"><span class="log-icon">{{ item.error ? '×' : '✓' }}</span><b>语句 {{ tab.outcomeOffset + index + 1 }}</b><span class="muted">行 {{ item.line }} · {{ item.elapsedMs }} ms</span><span class="spacer"></span><span>{{ item.results.length ? `${item.results.reduce((count, set) => count + set.totalRows, 0).toLocaleString()} 行返回` : `${item.affectedRows} 行受影响` }}</span><span v-if="item.warnings" class="warning-text">{{ item.warnings }} 个警告</span></div><pre>{{ item.sql }}</pre><p v-if="item.error" class="text-danger">{{ item.error.code }}{{ item.error.sqlState ? ` / ${item.error.sqlState}` : '' }} · {{ item.error.message }}</p></article></div>
      </div>
    </div>
    <footer class="result-footer"><span v-if="busy" class="spinner"></span><span :class="{ 'text-danger': tab.report?.error }">{{ tab.progress || '就绪' }}</span><span class="spacer"></span><span v-if="tab.inTransaction" class="warning-text">使用 COMMIT 或 ROLLBACK 结束事务</span><span v-else class="muted">独立 SQL 会话</span></footer>
  </section>
</template>
