<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { command } from '../api'
import { useDbm } from '../store'
import type { ConnectionProfile, FileContent } from '../types'
const props = defineProps<{ profile: ConnectionProfile | null }>()
const emit = defineEmits<{ close: []; saved: [profile: ConnectionProfile] }>()
const dbm = useDbm()
const form = reactive<ConnectionProfile>(props.profile ? { ...props.profile } : { id: crypto.randomUUID(), name: '', host: '127.0.0.1', port: 3306, username: 'root', database: null, tls: false, caPem: null, caName: null, rememberPassword: true })
const password = ref('')
const passwordChanged = ref(false)
const busy = ref(false)
const feedback = ref('')
const failed = ref(false)
const valid = computed(() => form.name.trim() && form.host.trim() && form.username.trim() && Number.isInteger(Number(form.port)) && Number(form.port) > 0 && Number(form.port) <= 65535)
function input() { return { profile: { ...form, name: form.name.trim(), host: form.host.trim(), username: form.username.trim(), port: Number(form.port), database: form.database?.trim() || null }, password: props.profile && !passwordChanged.value ? null : password.value } }
async function test() {
  busy.value = true; feedback.value = ''
  try { feedback.value = `连接成功 · ${await command<string>('test_connection', { input: input() })}`; failed.value = false }
  catch (error) { feedback.value = (error as { message: string }).message; failed.value = true }
  finally { busy.value = false }
}
async function save() {
  if (!valid.value) return
  busy.value = true
  try { const data = input(); const saved = await dbm.saveProfile(data.profile, data.password); emit('saved', saved) }
  catch (error) { feedback.value = (error as { message: string }).message; failed.value = true }
  finally { busy.value = false }
}
async function pickCa() {
  try { const file = await command<FileContent | null>('pick_ca'); if (file) { form.caPem = file.content; form.caName = file.name } }
  catch (error) { dbm.showError(error) }
}
</script>

<template>
  <div class="modal-backdrop" @mousedown.self="!busy && emit('close')">
    <section class="modal connection-modal" role="dialog" aria-modal="true" aria-labelledby="connection-title">
      <header class="modal-header"><div><span class="eyebrow">MYSQL CONNECTION</span><h2 id="connection-title">{{ profile ? '编辑连接' : '新建连接' }}</h2></div><button class="icon-button" aria-label="关闭" :disabled="busy" @click="emit('close')">×</button></header>
      <form @submit.prevent="save">
        <div class="modal-body form-grid">
          <label class="span-2">连接名称<input v-model="form.name" autofocus placeholder="例如：本地开发环境" required maxlength="100" /></label>
          <label>主机地址<input v-model="form.host" required placeholder="127.0.0.1" /></label>
          <label>端口<input v-model.number="form.port" type="number" min="1" max="65535" required /></label>
          <label>用户名<input v-model="form.username" required autocomplete="username" /></label>
          <label>密码<input v-model="password" type="password" :placeholder="profile?.rememberPassword ? '留空保留已保存的密码' : '请输入密码（可以为空）'" autocomplete="new-password" @input="passwordChanged = true" /></label>
          <label class="span-2">默认数据库 <span class="muted">可选</span><input v-model="form.database" placeholder="连接后也可以选择数据库" /></label>
          <label class="check-label span-2"><input v-model="form.rememberPassword" type="checkbox" /><span>记住密码（保存在配置文件中）</span></label>
          <p v-if="form.rememberPassword" class="helper span-2 password-storage-hint">密码以明文保存在本机 settings.json 中，下次连接自动使用。</p>
          <div class="form-divider span-2"></div>
          <label class="check-label span-2"><input v-model="form.tls" type="checkbox" /><span>启用 TLS 加密 <span class="muted">验证证书与主机名</span></span></label>
          <div v-if="form.tls" class="ca-picker span-2"><div><b>自定义 CA 证书</b><p>{{ form.caName || '使用系统受信任证书' }}</p></div><button type="button" class="button" @click="pickCa">选择 PEM 文件</button><button v-if="form.caPem" type="button" class="icon-button" title="移除证书" @click="form.caPem = null; form.caName = null">×</button></div>
          <p v-if="feedback" class="inline-feedback span-2" :class="{ 'text-danger': failed, 'text-success': !failed }">{{ feedback }}</p>
        </div>
        <footer class="modal-footer"><button type="button" class="button" :disabled="!valid || busy" @click="test">{{ busy ? '请稍候…' : '测试连接' }}</button><span class="spacer"></span><button type="button" class="button" :disabled="busy" @click="emit('close')">取消</button><button type="submit" class="button primary" :disabled="!valid || busy">保存连接</button></footer>
      </form>
    </section>
  </div>
</template>
