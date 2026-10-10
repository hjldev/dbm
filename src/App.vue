<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen } from '@tauri-apps/api/event'
import { nativeAvailable } from './api'
import { useDbm } from './store'
import type { ConnectionProfile, ObjectInfo } from './types'
import Icon from './components/Icon.vue'
import ConnectionDialog from './components/ConnectionDialog.vue'
import TablePanel from './components/TablePanel.vue'
import SqlPanel from './components/SqlPanel.vue'
const dbm = useDbm()
const search = ref('')
const connectionDialog = ref(false)
const editingProfile = ref<ConnectionProfile | null>(null)
const password = ref('')
const settingsOpen = ref(false)
const sqlPanels = new Map<string, { save: () => void; open: () => void }>()
const searchTerm = computed(() => search.value.trim().toLowerCase())
const native = nativeAvailable()
const connectionCount = computed(() => Object.keys(dbm.connected).length)
const selectedProfile = computed(() => dbm.connections.find(profile => profile.id === dbm.selectedConnection))
watch(() => dbm.settings.theme, theme => { document.documentElement.dataset.theme = theme }, { immediate: true })
watch(() => dbm.credentialConnection, () => { password.value = '' })
function addConnection() { editingProfile.value = null; connectionDialog.value = true }
async function editConnection(profile: ConnectionProfile) {
  if (dbm.connected[profile.id]) {
    if (!await dbm.ask('断开连接后编辑配置？', '连接配置需要在断开后编辑；此连接的查询和数据标签将关闭。', '断开并编辑')) return
    if (!await dbm.disconnect(profile.id)) return
  }
  editingProfile.value = profile; connectionDialog.value = true
}
function profileMatches(profile: ConnectionProfile) {
  if (!searchTerm.value || `${profile.name} ${profile.host}`.toLowerCase().includes(searchTerm.value)) return true
  return (dbm.databases[profile.id] ?? []).some(database => database.toLowerCase().includes(searchTerm.value) || (dbm.objects[`${profile.id}:${database}`] ?? []).some(object => object.name.toLowerCase().includes(searchTerm.value)))
}
function databaseMatches(connectionId: string, database: string) { return !searchTerm.value || database.toLowerCase().includes(searchTerm.value) || selectedProfile.value?.name.toLowerCase().includes(searchTerm.value) || (dbm.objects[`${connectionId}:${database}`] ?? []).some(object => object.name.toLowerCase().includes(searchTerm.value)) }
function filteredObjects(connectionId: string, database: string): ObjectInfo[] { const list = dbm.objects[`${connectionId}:${database}`] ?? []; return searchTerm.value && !database.toLowerCase().includes(searchTerm.value) ? list.filter(object => object.name.toLowerCase().includes(searchTerm.value)) : list }
function saveConnection(profile: ConnectionProfile) { connectionDialog.value = false; dbm.selectedConnection = profile.id; dbm.notify('连接配置已保存') }
function resizeSidebar(event: MouseEvent) {
  event.preventDefault()
  const move = (next: MouseEvent) => { dbm.settings.sidebarWidth = Math.min(400, Math.max(210, next.clientX)) }
  const stop = () => { window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', stop); if (native) void dbm.updateSettings({ sidebarWidth: dbm.settings.sidebarWidth }) }
  window.addEventListener('mousemove', move); window.addEventListener('mouseup', stop)
}
function shortcut(event: KeyboardEvent) {
  if (!(event.metaKey || event.ctrlKey)) return
  if (event.key.toLowerCase() === 'n') { event.preventDefault(); dbm.activeProfile && dbm.connected[dbm.activeProfile.id] ? dbm.newQuery() : addConnection() }
  if (!native && event.key.toLowerCase() === 'w' && dbm.activeTabId) { event.preventDefault(); void dbm.closeTab(dbm.activeTabId) }
  if (event.key.toLowerCase() === 's' && dbm.activeTab?.kind === 'sql') { event.preventDefault(); sqlPanels.get(dbm.activeTabId!)?.save() }
}
let unlistenClose: (() => void) | undefined
let unlistenCloseTab: (() => void) | undefined
let closing = false
onMounted(async () => {
  await dbm.init(); window.addEventListener('keydown', shortcut)
  if (native) {
    unlistenCloseTab = await listen('dbm-close-tab', () => { if (dbm.activeTabId) void dbm.closeTab(dbm.activeTabId) })
    unlistenClose = await getCurrentWindow().onCloseRequested(async event => {
      event.preventDefault(); if (closing) return; closing = true
      try { if (await dbm.canCloseWindow()) await getCurrentWindow().destroy() } catch (error) { dbm.showError(error) } finally { closing = false }
    })
  }
})
onBeforeUnmount(() => { unlistenClose?.(); unlistenCloseTab?.(); window.removeEventListener('keydown', shortcut) })
</script>

<template>
  <div class="app-shell" :style="{ '--sidebar-width': `${dbm.settings.sidebarWidth}px` }">
    <aside class="sidebar"><header class="brand"><div class="brand-mark"><Icon name="database" :size="21" /></div><div><b>DBM</b><span>MySQL 工作台</span></div><span class="version-tag">v0.1</span></header>
      <div class="sidebar-controls"><div class="sidebar-title"><span>连接</span><span class="spacer"></span><button class="icon-button" title="刷新当前连接" :disabled="!dbm.selectedConnection || !dbm.connected[dbm.selectedConnection]" @click="dbm.selectedConnection && dbm.refreshDatabases(dbm.selectedConnection)"><Icon name="refresh" :size="14" /></button><button class="icon-button" title="新建连接" aria-label="新建连接" @click="addConnection"><Icon name="plus" :size="16" /></button></div><label class="search-field"><Icon name="search" :size="14" /><input v-model="search" aria-label="搜索连接和已加载结构" placeholder="搜索连接、数据库、表" /><button v-if="search" class="icon-button" aria-label="清除搜索" @click="search = ''">×</button></label></div>
      <nav class="connection-tree" aria-label="数据库连接"><div v-for="profile in dbm.connections.filter(profileMatches)" :key="profile.id" class="connection-branch"><div class="tree-row connection-row" :class="{ chosen: dbm.selectedConnection === profile.id }"><button class="tree-main" :title="`${profile.username}@${profile.host}:${profile.port}`" @click="dbm.connect(profile)"><span v-if="dbm.connecting[profile.id]" class="spinner tiny"></span><Icon v-else name="chevron" :size="11" :class="{ expanded: dbm.expanded[profile.id] && dbm.connected[profile.id] }" /><Icon name="plug" :size="16" class="tree-icon" :class="{ 'text-success': dbm.connected[profile.id] }" /><span class="tree-label">{{ profile.name }}</span><span class="connection-status" :class="{ online: dbm.connected[profile.id] }"></span></button><div class="tree-actions"><button v-if="dbm.connected[profile.id]" class="icon-button" title="新建查询" @click="dbm.newQuery(profile.id, profile.database)"><Icon name="plus" :size="12" /></button><button class="icon-button" title="编辑连接" @click="editConnection(profile)"><Icon name="edit" :size="12" /></button><button v-if="dbm.connected[profile.id]" class="icon-button" title="断开连接" @click="dbm.disconnect(profile.id)"><Icon name="disconnect" :size="13" /></button><button v-else class="icon-button" title="删除连接" @click="dbm.deleteConnection(profile)"><Icon name="trash" :size="12" /></button></div></div>
        <div v-if="dbm.expanded[profile.id] && dbm.connected[profile.id]" class="database-branches"><div v-if="dbm.loading[profile.id]" class="tree-loading">加载数据库…</div><template v-for="database in dbm.databases[profile.id]" :key="database"><div v-if="databaseMatches(profile.id, database)" class="database-branch"><div class="tree-row database-row" :class="{ chosen: dbm.selectedConnection === profile.id && dbm.selectedDatabase === database }"><button class="tree-main" @click="dbm.expandDatabase(profile.id, database)"><Icon name="chevron" :size="10" :class="{ expanded: dbm.expanded[`${profile.id}:${database}`] || searchTerm }" /><Icon name="database" :size="14" class="tree-icon" /><span class="tree-label">{{ database }}</span></button><div class="tree-actions"><button class="icon-button" title="刷新结构" @click="dbm.expandDatabase(profile.id, database, true)"><Icon name="refresh" :size="12" /></button><button class="icon-button" title="在此数据库新建查询" @click="dbm.newQuery(profile.id, database)"><Icon name="plus" :size="12" /></button></div></div><div v-if="dbm.expanded[`${profile.id}:${database}`] || searchTerm" class="object-branches"><div v-if="dbm.loading[`${profile.id}:${database}`]" class="tree-loading">加载结构…</div><button v-for="object in filteredObjects(profile.id, database)" :key="object.name" class="tree-row object-row" :class="{ chosen: dbm.activeTab?.kind === 'table' && dbm.activeTab.connectionId === profile.id && dbm.activeTab.database === database && dbm.activeTab.table === object.name }" :title="object.comment || `${object.kind} · ${object.engine || 'VIEW'}`" @click="dbm.openTable(profile.id, database, object.name)"><Icon :name="object.kind === 'view' ? 'view' : 'table'" :size="13" class="tree-icon" /><span class="tree-label">{{ object.name }}</span><span v-if="object.kind === 'view'" class="view-mark">视图</span></button><div v-if="!dbm.loading[`${profile.id}:${database}`] && dbm.objects[`${profile.id}:${database}`]?.length === 0" class="tree-loading">没有表或视图</div></div></div></template></div>
      </div><div v-if="!dbm.connections.length" class="sidebar-empty"><Icon name="plug" :size="24" /><p>还没有数据库连接</p><button class="button small" @click="addConnection">＋ 新建连接</button></div><p v-else-if="search && !dbm.connections.some(profileMatches)" class="sidebar-empty">没有匹配的已加载结构</p></nav>
      <footer class="sidebar-footer"><span class="connection-status" :class="{ online: connectionCount }"></span><span>{{ connectionCount ? `${connectionCount} 个连接在线` : '尚未连接' }}</span><span class="spacer"></span><button class="icon-button" title="界面设置" aria-label="界面设置" @click="settingsOpen = true"><Icon name="settings" :size="16" /></button></footer><div class="sidebar-resizer" @mousedown="resizeSidebar"></div>
    </aside>
    <main class="main-workspace"><header class="topbar"><div class="toolbar-actions"><Icon name="code" :size="17" /><span>数据库工作台</span></div><span class="spacer"></span><button class="button small" @click="addConnection"><Icon name="plus" :size="13" />新建连接</button><button class="button small primary" :disabled="!dbm.activeProfile || !dbm.connected[dbm.activeProfile.id]" @click="dbm.newQuery()"><Icon name="code" :size="14" />新建查询</button></header>
      <div v-if="!native" class="native-banner">此页面是浏览器预览。数据库连接与文件操作需要在 DBM 桌面应用中运行。</div>
      <div v-if="dbm.tabs.length" class="workspace-tabs" role="tablist"><div v-for="tab in dbm.tabs" :key="tab.id" class="workspace-tab" :class="{ active: dbm.activeTabId === tab.id }" role="tab" :aria-selected="dbm.activeTabId === tab.id" @click="dbm.activeTabId = tab.id"><Icon :name="tab.kind === 'sql' ? 'code' : 'table'" :size="14" /><span>{{ tab.title }}</span><span v-if="tab.kind === 'table' ? tab.dirty : tab.source !== tab.savedSource" class="dirty-dot" title="尚未保存"></span><span v-if="tab.busy" class="spinner tiny"></span><button class="tab-close" :aria-label="`关闭 ${tab.title}`" @click.stop="dbm.closeTab(tab.id)">×</button></div><button class="tab-add" title="新建查询" @click="dbm.newQuery()">＋</button></div>
      <div v-if="dbm.tabs.length" class="workspace-body"><template v-for="tab in dbm.tabs" :key="tab.id"><SqlPanel v-if="tab.kind === 'sql'" v-show="dbm.activeTabId === tab.id" :ref="el => { if (el) sqlPanels.set(tab.id, el as unknown as { save: () => void; open: () => void }); else sqlPanels.delete(tab.id) }" :tab="tab" /><TablePanel v-else v-show="dbm.activeTabId === tab.id" :tab="tab" /></template></div>
      <section v-else class="welcome"><div class="welcome-decoration decoration-one"></div><div class="welcome-decoration decoration-two"></div><div class="welcome-content"><div class="welcome-icon"><Icon name="database" :size="42" /><span class="welcome-icon-dot"></span></div><span class="eyebrow">YOUR DATA, ONE CLEAR VIEW</span><h1>从一个连接开始。</h1><p>连接 MySQL，浏览数据，编写查询。<br />一个轻巧、专注的数据库工作台。</p><div class="welcome-actions"><button class="button primary" @click="addConnection"><Icon name="plus" :size="16" />新建数据库连接</button><button v-if="dbm.activeProfile && dbm.connected[dbm.activeProfile.id]" class="button" @click="dbm.newQuery()">打开 SQL 查询</button></div><div class="welcome-features"><div><Icon name="plug" :size="18" /><b>安全连接</b><span>TLS 与已保存凭据</span></div><div><Icon name="table" :size="18" /><b>流畅浏览</b><span>按需加载与分页数据</span></div><div><Icon name="code" :size="18" /><b>专注查询</b><span>独立会话与 SQL 补全</span></div></div></div><div class="welcome-bottom"><span>本地桌面工作台</span><span>MySQL 8.0 / 8.4</span></div></section>
    </main>
    <ConnectionDialog v-if="connectionDialog" :profile="editingProfile" @close="connectionDialog = false" @saved="saveConnection" />
    <div v-if="dbm.credentialConnection" class="modal-backdrop" @mousedown.self="dbm.credentialConnection = null"><section class="modal password-modal" role="dialog" aria-modal="true" aria-labelledby="password-title"><header class="modal-header"><div><span class="eyebrow">MYSQL CONNECTION</span><h2 id="password-title">连接 {{ dbm.credentialConnection.name }}</h2></div><button class="icon-button" aria-label="关闭" @click="dbm.credentialConnection = null">×</button></header><form @submit.prevent="dbm.connect(dbm.credentialConnection!, password)"><div class="modal-body"><p class="helper">{{ dbm.credentialConnection.username }}@{{ dbm.credentialConnection.host }}:{{ dbm.credentialConnection.port }}</p><label>密码<input v-model="password" type="password" autofocus autocomplete="current-password" placeholder="请输入连接密码" /></label><p class="helper">{{ dbm.credentialConnection.rememberPassword ? '连接成功后，密码将以明文保存在配置文件中。' : '密码仅在本次连接期间保留。' }}</p></div><footer class="modal-footer"><span class="spacer"></span><button type="button" class="button" @click="dbm.credentialConnection = null">取消</button><button type="submit" class="button primary" :disabled="dbm.connecting[dbm.credentialConnection.id]">{{ dbm.connecting[dbm.credentialConnection.id] ? '连接中…' : '连接' }}</button></footer></form></section></div>
    <div v-if="dbm.confirmDialog" class="modal-backdrop confirm-backdrop"><section class="modal confirm-modal" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title"><header class="modal-header"><h2 id="confirm-title">{{ dbm.confirmDialog.title }}</h2></header><div class="modal-body"><p>{{ dbm.confirmDialog.message }}</p></div><footer class="modal-footer"><span class="spacer"></span><button class="button" @click="dbm.answerConfirm(false)">取消</button><button class="button" :class="dbm.confirmDialog.danger ? 'danger' : 'primary'" @click="dbm.answerConfirm(true)">{{ dbm.confirmDialog.action }}</button></footer></section></div>
    <div v-if="settingsOpen" class="modal-backdrop" @mousedown.self="settingsOpen = false"><section class="modal settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title"><header class="modal-header"><h2 id="settings-title">界面设置</h2><button class="icon-button" aria-label="关闭" @click="settingsOpen = false">×</button></header><div class="modal-body"><label>外观<select :value="dbm.settings.theme" @change="dbm.updateSettings({ theme: ($event.target as HTMLSelectElement).value as 'system' | 'light' | 'dark' })"><option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option></select></label><div v-if="dbm.configPath"><p class="helper">配置文件位置</p><code class="config-path">{{ dbm.configPath }}</code><p class="helper">记住的密码以明文保存，可在退出 DBM 后管理此文件。</p></div><p class="helper">DBM v0.1.0 · MySQL 8.0 / 8.4<br />⌘ N 新建查询 · ⌘ W 关闭标签 · ⌘ S 保存 SQL</p></div></section></div>
    <div class="toast-stack" aria-live="polite"><div v-for="notice in dbm.notices" :key="notice.id" class="toast" :class="{ error: notice.error }"><span>{{ notice.error ? '!' : '✓' }}</span><p>{{ notice.text }}</p><button class="icon-button" aria-label="关闭提示" @click="dbm.notices = dbm.notices.filter(item => item.id !== notice.id)">×</button></div></div>
  </div>
</template>
