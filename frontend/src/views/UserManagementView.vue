<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { NButton, NInput, NModal, NSelect, NSwitch, useDialog, useMessage } from 'naive-ui'
import { api } from '@/api/client'

interface UserSummary {
  id: string
  username: string
  displayName: string | null
  role: 'admin' | 'user'
  status: 'active' | 'disabled'
  mustChangePassword: boolean
  providerCount: number
  taskCount: number
  imageBytes: number
  lastLoginAt: string | null
}

const users = ref<UserSummary[]>([])
const createOpen = ref(false)
const username = ref('')
const displayName = ref('')
const password = ref('')
const role = ref<'admin' | 'user'>('user')
const message = useMessage()
const dialog = useDialog()

onMounted(load)

async function load() {
  users.value = await api('/api/v1/admin/users')
}

async function create() {
  try {
    await api('/api/v1/admin/users', { method: 'POST', body: JSON.stringify({ username: username.value, displayName: displayName.value || null, role: role.value, password: password.value }) })
    createOpen.value = false
    username.value = displayName.value = password.value = ''
    await load()
    message.success('用户已创建')
  } catch (error) { message.error(error instanceof Error ? error.message : '创建失败') }
}

async function update(item: UserSummary, payload: { role?: string; status?: string }) {
  try {
    await api(`/api/v1/admin/users/${item.id}`, { method: 'PATCH', body: JSON.stringify(payload) })
    await load()
  } catch (error) { message.error(error instanceof Error ? error.message : '更新失败'); await load() }
}

function reset(item: UserSummary) {
  dialog.warning({
    title: '重置密码', content: `确认重置 ${item.username} 的密码？现有会话将失效。`, positiveText: '确认重置', negativeText: '取消',
    onPositiveClick: async () => {
      const result = await api<{ temporaryPassword: string }>(`/api/v1/admin/users/${item.id}/reset-password`, { method: 'POST' })
      dialog.success({ title: '一次性临时密码', content: result.temporaryPassword, positiveText: '我已保存' })
      await load()
    },
  })
}
</script>

<template>
  <div class="page">
    <header class="page-header"><div><span class="eyebrow muted">ADMINISTRATION</span><h1>用户管理</h1><p>管理账号、角色和访问状态，不展示用户 Provider 凭据。</p></div><n-button type="primary" @click="createOpen = true">创建用户</n-button></header>
    <div class="table-panel panel">
      <table><thead><tr><th>用户</th><th>角色</th><th>状态</th><th>Provider</th><th>任务</th><th>图片占用</th><th>最后登录</th><th>操作</th></tr></thead>
        <tbody><tr v-for="item in users" :key="item.id">
          <td><strong>{{ item.displayName || item.username }}</strong><small>@{{ item.username }}<i v-if="item.role === 'admin' && item.mustChangePassword">待改密</i></small></td>
          <td><n-select size="small" :value="item.role" :options="[{label:'管理员',value:'admin'},{label:'用户',value:'user'}]" @update:value="value => update(item, { role: value })" /></td>
          <td><n-switch :value="item.status === 'active'" @update:value="value => update(item, { status: value ? 'active' : 'disabled' })" /></td>
          <td>{{ item.providerCount }}</td><td>{{ item.taskCount }}</td><td>{{ (item.imageBytes / 1024 / 1024).toFixed(1) }} MB</td><td>{{ item.lastLoginAt ? new Date(item.lastLoginAt).toLocaleString() : '从未登录' }}</td>
          <td><n-button size="small" @click="reset(item)">重置密码</n-button></td>
        </tr></tbody>
      </table>
    </div>
  </div>
  <n-modal v-model:show="createOpen" preset="card" title="创建用户" class="dialog-card"><div class="form-stack">
    <label>用户名<n-input v-model:value="username" /></label><label>显示名称<n-input v-model:value="displayName" /></label>
    <label>初始密码<n-input v-model:value="password" type="password" show-password-on="click" /></label><label>角色<n-select v-model:value="role" :options="[{label:'用户',value:'user'},{label:'管理员',value:'admin'}]" /></label>
    <n-button type="primary" @click="create">创建</n-button>
  </div></n-modal>
</template>
