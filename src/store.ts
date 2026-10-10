import { computed, ref, shallowReactive, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { Channel, command, nativeAvailable, normalizeError } from './api'
import type { BootstrapData, ConnectionProfile, ExecutionEvent, ExecutionReport, ObjectInfo, Settings, SqlTab, WorkspaceTab } from './types'

interface ConfirmDialog { title: string; message: string; action: string; danger: boolean; resolve: (answer: boolean) => void }
export const useDbm = defineStore('dbm', () => {
  const connections = ref<ConnectionProfile[]>([])
  const settings = ref<Settings>({ theme: 'system', previewLimit: 1000, sidebarWidth: 250 })
  const configPath = ref('')
  const tabs = shallowRef<WorkspaceTab[]>([])
  const activeTabId = ref<string | null>(null)
  const selectedConnection = ref<string | null>(null)
  const selectedDatabase = ref<string | null>(null)
  const connected = ref<Record<string, string>>({})
  const connecting = ref<Record<string, boolean>>({})
  const databases = ref<Record<string, string[]>>({})
  const objects = ref<Record<string, ObjectInfo[]>>({})
  const loading = ref<Record<string, boolean>>({})
  const expanded = ref<Record<string, boolean>>({})
  const confirmDialog = shallowRef<ConfirmDialog | null>(null)
  const credentialConnection = ref<ConnectionProfile | null>(null)
  const notices = ref<{ id: string; text: string; error: boolean }[]>([])
  const ready = ref(false)
  const activeTab = computed(() => tabs.value.find(tab => tab.id === activeTabId.value) ?? null)
  const activeProfile = computed(() => connections.value.find(profile => profile.id === (activeTab.value?.connectionId ?? selectedConnection.value)) ?? null)
  const recentResults: { tabId: string; outcomeIndex: number; resultIndex: number }[] = []
  let queryNumber = 0

  function notify(text: string, error = false) {
    const id = crypto.randomUUID(); notices.value.push({ id, text, error })
    setTimeout(() => { notices.value = notices.value.filter(notice => notice.id !== id) }, error ? 10000 : 5000)
  }
  function showError(error: unknown) { notify(normalizeError(error).message, true) }
  function ask(title: string, message: string, action = '继续', danger = false): Promise<boolean> {
    if (confirmDialog.value) return Promise.resolve(false)
    return new Promise(resolve => { confirmDialog.value = { title, message, action, danger, resolve } })
  }
  function answerConfirm(answer: boolean) { const dialog = confirmDialog.value; confirmDialog.value = null; dialog?.resolve(answer) }

  async function init() {
    if (nativeAvailable()) {
      try { const data = await command<BootstrapData>('bootstrap'); connections.value = data.connections; settings.value = data.settings; configPath.value = data.configPath }
      catch (error) { showError(error) }
    }
    ready.value = true
  }
  async function updateSettings(patch: Partial<Settings>) {
    const updated = { ...settings.value, ...patch }
    try { await command('save_settings', { settings: updated }); settings.value = updated }
    catch (error) { showError(error) }
  }
  async function refreshDatabases(connectionId: string) {
    loading.value[connectionId] = true
    try { databases.value[connectionId] = await command<string[]>('list_databases', { connectionId }) }
    catch (error) { showError(error) }
    finally { loading.value[connectionId] = false }
  }
  async function connect(profile: ConnectionProfile, password?: string) {
    selectedConnection.value = profile.id
    if (connected.value[profile.id]) { expanded.value[profile.id] = !expanded.value[profile.id]; return }
    connecting.value[profile.id] = true
    try {
      const server = await command<string>('connect', { connectionId: profile.id, password: password ?? null })
      connected.value[profile.id] = server; expanded.value[profile.id] = true
      selectedDatabase.value = profile.database; credentialConnection.value = null
      await refreshDatabases(profile.id)
      notify(`已连接 ${profile.name}`)
    } catch (error) {
      const issue = normalizeError(error)
      if (issue.code === 'PASSWORD_REQUIRED') credentialConnection.value = profile
      else showError(error)
    } finally { connecting.value[profile.id] = false }
  }
  async function expandDatabase(connectionId: string, database: string, refresh = false) {
    selectedConnection.value = connectionId; selectedDatabase.value = database
    const key = `${connectionId}:${database}`
    if (!refresh) expanded.value[key] = !expanded.value[key]
    if (!expanded.value[key] && !refresh) return
    if (objects.value[key] && !refresh) return
    loading.value[key] = true
    try { objects.value[key] = await command<ObjectInfo[]>('list_objects', { connectionId, database }) }
    catch (error) { showError(error) }
    finally { loading.value[key] = false }
  }
  function newQuery(connectionId = activeProfile.value?.id, database?: string | null) {
    if (!connectionId || !connected.value[connectionId]) { notify('请先连接一个 MySQL 数据库', true); return }
    const initialDatabase = database !== undefined ? database : selectedConnection.value === connectionId ? selectedDatabase.value : activeTab.value?.connectionId === connectionId ? activeTab.value.database : connections.value.find(profile => profile.id === connectionId)?.database ?? null
    const tab: SqlTab = shallowReactive({ id: crypto.randomUUID(), kind: 'sql', connectionId, database: initialDatabase, title: `查询 ${++queryNumber}`, source: '', savedSource: '', fileName: null, sessionId: null, inTransaction: false, sessionValid: true, busy: false, executionId: null, outcomes: [], selectedOutcome: 0, selectedResult: 0, progress: '', report: null })
    tabs.value = [...tabs.value, tab]; activeTabId.value = tab.id
  }
  function openTable(connectionId: string, database: string, table: string) {
    const existing = tabs.value.find(tab => tab.kind === 'table' && tab.connectionId === connectionId && tab.database === database && tab.table === table)
    if (existing) { activeTabId.value = existing.id; return }
    const tab = shallowReactive({ id: crypto.randomUUID(), kind: 'table' as const, connectionId, database, table, title: table, dirty: false, busy: false })
    tabs.value = [...tabs.value, tab]; activeTabId.value = tab.id
  }
  async function ensureSession(tab: SqlTab) {
    if (!tab.sessionValid) throw { code: 'SESSION_LOST', message: '此 SQL 会话已失效，请新建查询。原 SQL 可另存为文件。' }
    if (!tab.sessionId) tab.sessionId = await command<string>('open_session', { connectionId: tab.connectionId, database: tab.database })
    return tab.sessionId
  }
  function trimPreviews(tab: SqlTab, index: number) {
    for (let resultIndex = 0; resultIndex < tab.outcomes[index]!.results.length; resultIndex++) recentResults.push({ tabId: tab.id, outcomeIndex: index, resultIndex })
    while (recentResults.length > 10) {
      const oldest = recentResults.shift()!
      const target = tabs.value.find(item => item.id === oldest.tabId)
      if (target?.kind !== 'sql') continue
      target.outcomes = target.outcomes.map((outcome, i) => i === oldest.outcomeIndex ? { ...outcome, results: outcome.results.map((result, resultIndex) => resultIndex === oldest.resultIndex ? { ...result, rows: [], truncated: result.totalRows > 0 } : result) } : outcome)
    }
  }
  async function execute(tab: SqlTab, source: string) {
    if (tab.busy || !source.trim()) return
    tab.busy = true; tab.progress = '正在准备会话…'; tab.report = null
    try {
      const sessionId = await ensureSession(tab)
      tab.executionId = crypto.randomUUID()
      const events = new Channel<ExecutionEvent>()
      events.onmessage = event => {
        if (event.type === 'progress') tab.progress = `正在执行第 ${event.index + 1} 条语句 · 行 ${event.line}`
        else {
          const index = tab.outcomes.length
          tab.outcomes = [...tab.outcomes, event.outcome]
          tab.selectedOutcome = index; tab.selectedResult = 0
          if (event.outcome.results.length) trimPreviews(tab, index)
        }
      }
      const report = await command<ExecutionReport>('execute_script', { sessionId, executionId: tab.executionId, source, previewLimit: settings.value.previewLimit, events })
      tab.report = report; tab.inTransaction = report.inTransaction; tab.sessionValid = report.sessionValid; tab.database = report.database
      if (report.error) showError(report.error)
      tab.progress = report.cancelled ? '已取消执行' : report.error ? '遇到错误，后续语句未执行' : `执行完成 · ${report.completed} 条语句`
    } catch (error) { const issue = normalizeError(error); if (['SESSION_LOST', 'CONNECTION', 'SESSION_INVALID'].includes(issue.code)) tab.sessionValid = false; showError(error); tab.progress = '执行失败' }
    finally { tab.busy = false; tab.executionId = null }
  }
  async function cancel(executionId: string | null) { if (!executionId) return; try { await command('cancel_execution', { executionId }) } catch (error) { showError(error) } }
  function clearResults(tab: SqlTab) {
    if (tab.busy) return
    tab.outcomes = []; tab.selectedOutcome = 0; tab.selectedResult = 0; tab.report = null; tab.progress = ''
    for (let i = recentResults.length - 1; i >= 0; i--) if (recentResults[i]!.tabId === tab.id) recentResults.splice(i, 1)
  }
  async function canDiscard(tab: WorkspaceTab) {
    if (tab.busy) { notify('操作仍在进行，请先取消并等待结束', true); return false }
    if (tab.kind === 'table' && tab.dirty && !await ask('放弃未保存的数据变更？', `${tab.title} 中的新增、修改与删除尚未提交。`, '放弃变更', true)) return false
    if (tab.kind === 'sql' && tab.source !== tab.savedSource && !await ask('关闭未保存的 SQL？', `${tab.title} 中的 SQL 尚未保存到文件。`, '关闭查询', true)) return false
    return true
  }
  async function closeTab(id: string) {
    const tab = tabs.value.find(item => item.id === id)
    if (!tab || !await canDiscard(tab)) return
    if (tab.kind === 'sql' && tab.sessionId) {
      try { await command('close_session', { sessionId: tab.sessionId, rollback: false }) }
      catch (error) {
        if (normalizeError(error).code !== 'TRANSACTION_OPEN') { showError(error); return }
        if (!await ask('回滚事务并关闭查询？', '此会话存在未提交事务，关闭后将回滚未提交的数据库修改。', '回滚并关闭', true)) return
        try { await command('close_session', { sessionId: tab.sessionId, rollback: true }) } catch (issue) { showError(issue); return }
      }
    }
    removeTabs([id])
  }
  function removeTabs(ids: string[]) {
    const oldIndex = tabs.value.findIndex(tab => tab.id === activeTabId.value)
    tabs.value = tabs.value.filter(tab => !ids.includes(tab.id))
    if (ids.includes(activeTabId.value ?? '')) activeTabId.value = tabs.value[Math.min(oldIndex, tabs.value.length - 1)]?.id ?? null
    for (let i = recentResults.length - 1; i >= 0; i--) if (ids.includes(recentResults[i]!.tabId)) recentResults.splice(i, 1)
  }
  async function disconnect(connectionId: string) {
    const related = tabs.value.filter(tab => tab.connectionId === connectionId)
    for (const tab of related) if (!await canDiscard(tab)) return false
    let rollback = false
    try { await command('disconnect', { connectionId, rollback }) }
    catch (error) {
      if (normalizeError(error).code !== 'TRANSACTION_OPEN') { showError(error); return false }
      if (!await ask('回滚事务并断开连接？', '此连接中的 SQL 会话存在未提交事务，断开连接会回滚事务。', '回滚并断开', true)) return false
      rollback = true
      try { await command('disconnect', { connectionId, rollback }) } catch (issue) { showError(issue); return false }
    }
    removeTabs(related.map(tab => tab.id)); delete connected.value[connectionId]; delete databases.value[connectionId]
    for (const key of Object.keys(objects.value)) if (key.startsWith(`${connectionId}:`)) delete objects.value[key]
    if (selectedConnection.value === connectionId) selectedDatabase.value = null
    return true
  }
  async function deleteConnection(profile: ConnectionProfile) {
    if (!await ask('删除此连接？', `将删除 ${profile.name} 的连接配置及保存的密码。`, '删除连接', true)) return
    if (connected.value[profile.id] && !await disconnect(profile.id)) return
    try { await command('delete_connection', { connectionId: profile.id }); connections.value = connections.value.filter(item => item.id !== profile.id) }
    catch (error) { showError(error) }
  }
  async function saveProfile(profile: ConnectionProfile, password: string | null) {
    const saved = await command<ConnectionProfile>('save_connection', { input: { profile, password } })
    const existing = connections.value.findIndex(item => item.id === saved.id)
    if (existing >= 0) connections.value.splice(existing, 1, saved); else connections.value.push(saved)
    return saved
  }
  async function canCloseWindow() {
    for (const tab of [...tabs.value]) {
      await closeTab(tab.id)
      if (tabs.value.some(item => item.id === tab.id)) return false
    }
    return true
  }
  return { connections, settings, configPath, tabs, activeTabId, activeTab, activeProfile, selectedConnection, selectedDatabase, connected, connecting, databases, objects, expanded, loading, confirmDialog, credentialConnection, notices, ready, init, updateSettings, notify, showError, ask, answerConfirm, connect, disconnect, deleteConnection, refreshDatabases, expandDatabase, newQuery, openTable, ensureSession, execute, cancel, clearResults, closeTab, saveProfile, canCloseWindow }
})
