<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { NButton, NInput, NModal, NSelect, useDialog, useMessage } from 'naive-ui'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'
import type { ThemePreference } from '@/types/api'

const auth = useAuthStore()
const theme = useThemeStore()
const route = useRoute()
const router = useRouter()
const message = useMessage()
const dialog = useDialog()
const accountWrap = ref<HTMLElement | null>(null)
const accountOpen = ref(false)
const profileOpen = ref(false)
const profileTheme = ref<ThemePreference>(auth.user?.themePreference ?? 'system')
const savingProfile = ref(false)
const requiresPasswordChange = computed(
  () => auth.user?.role === 'admin' && auth.user.mustChangePassword,
)
const passwordOpen = ref(requiresPasswordChange.value)
const currentPassword = ref('')
const newPassword = ref('')
const confirmPassword = ref('')
const savingPassword = ref(false)

const navigation = computed(() => [
  { to: '/studio', icon: '✦', label: '创作台' },
  { to: '/history', icon: '▦', label: '历史作品' },
  { to: '/usage', icon: '◫', label: '用量' },
  { to: '/providers', icon: '◇', label: 'Provider' },
  ...(auth.user?.role === 'admin'
    ? [
        { to: '/admin/users', icon: '♙', label: '用户管理' },
        { to: '/admin/operations', icon: '⌁', label: '运营监控' },
        { to: '/settings/storage', icon: '⚙', label: '系统设置' },
        { to: '/settings/updates', icon: '↥', label: '版本升级' },
      ]
    : []),
])

onMounted(() => {
  document.addEventListener('pointerdown', closeAccountOutside)
  document.addEventListener('keydown', closeAccountOnEscape)
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', closeAccountOutside)
  document.removeEventListener('keydown', closeAccountOnEscape)
})

function closeAccountOutside(event: PointerEvent) {
  if (accountOpen.value && !accountWrap.value?.contains(event.target as Node)) accountOpen.value = false
}

function closeAccountOnEscape(event: KeyboardEvent) {
  if (event.key === 'Escape') accountOpen.value = false
}

function openProfile() {
  profileTheme.value = auth.user?.themePreference ?? 'system'
  accountOpen.value = false
  profileOpen.value = true
}

function openPassword() {
  accountOpen.value = false
  passwordOpen.value = true
}

async function saveProfile() {
  savingProfile.value = true
  try {
    await auth.setTheme(profileTheme.value)
    profileOpen.value = false
    message.success('个人设置已保存')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '个人设置保存失败')
  } finally {
    savingProfile.value = false
  }
}

async function toggleTheme() {
  const preference = theme.resolved === 'dark' ? 'light' : 'dark'
  try {
    await auth.setTheme(preference)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '主题保存失败')
  }
}

async function openUserManagement() {
  accountOpen.value = false
  await router.push('/admin/users')
}

async function savePassword() {
  if (newPassword.value.length < 8) return message.error('新密码至少需要 8 个字符')
  if (newPassword.value !== confirmPassword.value) return message.error('两次输入的新密码不一致')
  savingPassword.value = true
  try {
    await auth.changePassword(currentPassword.value, newPassword.value)
    passwordOpen.value = false
    currentPassword.value = newPassword.value = confirmPassword.value = ''
    message.success('密码已修改')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '修改密码失败')
  } finally {
    savingPassword.value = false
  }
}

function confirmLogout() {
  accountOpen.value = false
  dialog.warning({
    title: '退出登录',
    content: '确认退出当前账号？尚未发送的创作内容不会保存。',
    positiveText: '确认退出',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await auth.logout()
        await router.push('/login')
      } catch (error) {
        message.error(error instanceof Error ? error.message : '退出登录失败')
      }
    },
  })
}
</script>

<template>
  <div class="app-shell">
    <button
      class="app-theme-toggle"
      :class="{ 'studio-theme-toggle': route.path === '/studio' }"
      type="button"
      aria-label="切换主题"
      :title="theme.resolved === 'dark' ? '切换浅色主题' : '切换深色主题'"
      @click="toggleTheme"
    >
      <span aria-hidden="true">{{ theme.resolved === 'dark' ? '☀' : '☾' }}</span>
      <span>{{ theme.resolved === 'dark' ? 'Light' : 'Dark' }}</span>
    </button>
    <aside class="main-nav">
      <div class="brand-mark">A</div>
      <nav class="nav-items" aria-label="主导航">
        <router-link
          v-for="item in navigation"
          :key="item.to"
          :to="item.to"
          class="nav-button"
          :class="{ active: route.path === item.to }"
          :title="item.label"
        >
          <span class="nav-icon">{{ item.icon }}</span>
          <span>{{ item.label }}</span>
        </router-link>
      </nav>
      <div ref="accountWrap" class="account-wrap">
        <button class="avatar-button" title="账户菜单" @click="accountOpen = !accountOpen">
          {{ auth.user?.username.slice(0, 1).toUpperCase() }}
        </button>
        <div v-if="accountOpen" class="account-menu">
          <div class="account-summary">
            <strong>{{ auth.user?.displayName || auth.user?.username }}</strong>
            <small>{{ auth.user?.role === 'admin' ? '管理员' : '用户' }}</small>
          </div>
          <button @click="openProfile">个人设置</button>
          <button @click="toggleTheme">
            {{ theme.resolved === 'dark' ? '切换浅色主题' : '切换深色主题' }}
          </button>
          <button @click="openPassword">修改密码</button>
          <button v-if="auth.user?.role === 'admin'" @click="openUserManagement">用户管理</button>
          <button class="danger" @click="confirmLogout">退出登录</button>
        </div>
      </div>
    </aside>
    <main class="route-content"><router-view /></main>
  </div>

  <n-modal v-model:show="profileOpen" preset="card" title="个人设置" class="dialog-card">
    <div class="profile-summary">
      <div><span>用户名</span><strong>{{ auth.user?.username }}</strong></div>
      <div><span>显示名称</span><strong>{{ auth.user?.displayName || '未设置' }}</strong></div>
      <div><span>角色</span><strong>{{ auth.user?.role === 'admin' ? '管理员' : '用户' }}</strong></div>
    </div>
    <div class="form-stack">
      <label>主题偏好<n-select v-model:value="profileTheme" :options="[{ label: '跟随系统', value: 'system' }, { label: '浅色', value: 'light' }, { label: '深色', value: 'dark' }]" /></label>
      <n-button type="primary" :loading="savingProfile" @click="saveProfile">保存个人设置</n-button>
    </div>
  </n-modal>

  <n-modal
    v-model:show="passwordOpen"
    preset="card"
    title="修改密码"
    class="dialog-card"
    :mask-closable="!requiresPasswordChange"
    :close-on-esc="!requiresPasswordChange"
    :closable="!requiresPasswordChange"
  >
    <p v-if="requiresPasswordChange" class="security-notice">
      默认管理员密码必须修改后才能继续使用其他功能。
    </p>
    <div class="form-stack">
      <label>当前密码<n-input v-model:value="currentPassword" type="password" show-password-on="click" /></label>
      <label>新密码<n-input v-model:value="newPassword" type="password" show-password-on="click" /></label>
      <label>确认新密码<n-input v-model:value="confirmPassword" type="password" show-password-on="click" /></label>
      <n-button type="primary" :loading="savingPassword" @click="savePassword">保存新密码</n-button>
    </div>
  </n-modal>
</template>
