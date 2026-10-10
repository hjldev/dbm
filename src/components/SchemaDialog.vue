<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { command, normalizeError } from '../api'
import { useDbm } from '../store'
import type { SchemaChangeRequest, SchemaDefinition, SchemaTarget, TableTab } from '../types'

const props = defineProps<{ target: SchemaTarget; tab: TableTab }>()
const emit = defineEmits<{ close: []; saved: []; refresh: [] }>()
const dbm = useDbm()
const form = reactive<SchemaDefinition>({ name: props.target.originalName, definition: '', indexKind: null })
const initial = ref('')
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const sql = ref('')
const deleteAcknowledged = ref(false)
const field = computed(() => props.target.kind === 'field')
const dropping = computed(() => props.target.action === 'drop')
const actionLabel = computed(() => ({ add: '新增', modify: '修改', drop: '删除' })[props.target.action])
const title = computed(() => `${actionLabel.value}${field.value ? '字段' : '索引'}`)
const serialized = computed(() => JSON.stringify(form))
const changed = computed(() => !dropping.value && !!initial.value && serialized.value !== initial.value)
const valid = computed(() => !!initial.value && (dropping.value || (!!form.name.trim() && !!form.definition.trim() && changed.value)))
watch(serialized, () => { sql.value = ''; error.value = ''; props.tab.schemaDirty = changed.value }, { flush: 'sync' })
watch(sql, () => { deleteAcknowledged.value = false }, { flush: 'sync' })
watch(() => form.indexKind, kind => { if (kind === 'primary') form.name = 'PRIMARY' })
function request(): SchemaChangeRequest { return { target: props.target, value: { ...form, name: form.name.trim(), definition: form.definition.trim() } } }
async function load() {
  loading.value = true; error.value = ''; props.tab.busy = true
  try { Object.assign(form, await command<SchemaDefinition>('schema_definition', { target: props.target })); initial.value = serialized.value; props.tab.schemaDirty = false }
  catch (issue) { error.value = normalizeError(issue).message }
  finally { loading.value = false; props.tab.busy = false }
}
async function close(refresh = false) {
  if (busy.value || loading.value) return
  if (changed.value && !await dbm.ask('放弃未保存的结构变更？', '字段或索引的修改尚未执行。', '放弃变更', true)) return
  props.tab.schemaDirty = false
  if (refresh) emit('refresh'); else emit('close')
}
async function preview() {
  if (!valid.value || busy.value || loading.value) return
  busy.value = true; props.tab.busy = true; error.value = ''
  try { sql.value = await command<string>('preview_schema_change', { request: request() }) }
  catch (issue) { error.value = normalizeError(issue).message }
  finally { busy.value = false; props.tab.busy = false }
}
async function save() {
  if (!sql.value || busy.value || !valid.value || (dropping.value && !deleteAcknowledged.value)) return
  busy.value = true; props.tab.busy = true; error.value = ''
  let saved = false
  try {
    await command('apply_schema_change', { request: request(), expectedSql: sql.value })
    props.tab.schemaDirty = false
    dbm.notify(`${field.value ? '字段' : '索引'}已${actionLabel.value}`)
    saved = true
  } catch (issue) { error.value = normalizeError(issue).message; sql.value = '' }
  finally { busy.value = false; props.tab.busy = false }
  if (saved) emit('saved')
}
function escape(event: KeyboardEvent) { if (event.key === 'Escape' && !dbm.confirmDialog) { event.preventDefault(); void close() } }
onMounted(() => { void load(); window.addEventListener('keydown', escape) })
onBeforeUnmount(() => { props.tab.schemaDirty = false; window.removeEventListener('keydown', escape) })
</script>

<template>
  <div class="modal-backdrop" @mousedown.self="close()">
    <section class="modal schema-modal" role="dialog" aria-modal="true" aria-labelledby="schema-title">
      <header class="modal-header"><div><span class="eyebrow">{{ target.database }} / {{ target.table }}</span><h2 id="schema-title">{{ title }}{{ target.originalName ? ` · ${target.originalName}` : '' }}</h2></div><button class="icon-button" aria-label="关闭结构编辑" :disabled="busy || loading" @click="close()">×</button></header>
      <form @submit.prevent="sql ? save() : preview()">
        <div class="modal-body">
          <div v-if="loading" class="schema-loading"><span class="spinner"></span>正在读取完整定义…</div>
          <div v-else-if="initial && dropping" class="schema-delete-summary"><p>即将删除{{ field ? '字段' : '索引' }} <strong class="mono">{{ target.originalName }}</strong></p><pre>{{ form.definition }}</pre><p class="helper warning-text">{{ field ? '删除字段会永久移除此字段及其所有数据，相关索引可能随之改变。' : '删除索引可能影响查询性能；删除主键或唯一索引还可能影响数据编辑。' }}执行后无法通过“撤销”恢复。</p></div>
          <fieldset v-else-if="initial" class="schema-form" :disabled="busy">
            <div class="form-grid">
              <label :class="{ 'span-2': field }">{{ field ? '字段名称' : '索引名称' }}<input v-model="form.name" :disabled="form.indexKind === 'primary'" required maxlength="64" autofocus /></label>
              <label v-if="!field">索引类型<select v-model="form.indexKind"><option value="primary">主键</option><option value="unique">唯一索引</option><option value="index">普通索引</option><option value="fulltext">全文索引</option><option value="spatial">空间索引</option></select></label>
              <label class="span-2">{{ field ? '字段定义' : '索引定义' }}<textarea v-model="form.definition" class="schema-input mono" :aria-label="field ? '字段定义' : '索引定义'" required spellcheck="false" /></label>
            </div>
            <p v-if="field" class="helper">定义包含类型、NULL、DEFAULT、自增、注释等完整属性。字符串默认值需加单引号，例如：<code>varchar(100) NOT NULL DEFAULT '' COMMENT '名称'</code>。请保留需要的原有属性。</p>
            <p v-else class="helper">按括号内字段顺序构建索引；可设置前缀长度、ASC / DESC 和索引选项，例如：<code>(`name`(20), `id` DESC)</code>。表达式索引也会完整回填。</p>
          </fieldset>
          <p v-if="error" class="inline-feedback text-danger" role="alert">{{ error }}</p>
          <div v-if="sql" class="schema-preview"><p class="small muted">将执行以下 SQL</p><pre>{{ sql }};</pre><p v-if="!dropping" class="helper warning-text">执行后立即生效，可能重建表或锁表；字段类型转换可能影响现有数据，结构修改无法通过“撤销”回滚。</p><label v-else class="check-label"><input v-model="deleteAcknowledged" type="checkbox" :disabled="busy" /><span>{{ field ? '我确认删除此字段及其数据' : '我确认删除此索引' }}</span></label></div>
        </div>
        <footer class="modal-footer"><button v-if="error" type="button" class="button small" :disabled="busy || loading" @click="close(true)">关闭并刷新结构</button><span class="spacer"></span><button type="button" class="button" :disabled="busy || loading" @click="close()">取消</button><button type="submit" class="button" :class="sql ? 'danger' : 'primary'" :disabled="!valid || busy || loading || (!!sql && dropping && !deleteAcknowledged)">{{ busy ? '请稍候…' : sql ? `执行${actionLabel}` : '预览 SQL' }}</button></footer>
      </form>
    </section>
  </div>
</template>
