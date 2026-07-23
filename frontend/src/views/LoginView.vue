<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { NButton, NInput, useMessage } from 'naive-ui'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'

const username = ref('admin')
const password = ref('123456')
const loading = ref(false)
const auth = useAuthStore()
const theme = useThemeStore()
const router = useRouter()
const message = useMessage()

async function submit() {
  loading.value = true
  try {
    await auth.login(username.value, password.value)
    await router.push('/studio')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '登录失败')
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="login-page">
    <button class="theme-fab" @click="theme.toggle()">{{ theme.resolved === 'dark' ? '☀' : '☾' }}</button>
    <section class="login-visual">
      <div class="visual-orb orb-one"></div>
      <div class="visual-orb orb-two"></div>
      <div class="visual-copy">
        <span class="eyebrow">AI IMAGE STUDIO</span>
        <h1>把每一次灵感<br />延续成一段创作。</h1>
        <p>多轮对话、动态模型、流式进度和真实图片持久化，都在同一个工作空间完成。</p>
      </div>
    </section>
    <section class="login-panel">
      <form class="login-card" @submit.prevent="submit">
        <div class="brand-mark large">A</div>
        <div>
          <h2>欢迎回来</h2>
          <p>登录 AI Image Studio 继续创作</p>
        </div>
        <label>用户名<n-input v-model:value="username" size="large" autocomplete="username" /></label>
        <label>密码<n-input v-model:value="password" size="large" type="password" show-password-on="click" autocomplete="current-password" /></label>
        <n-button attr-type="submit" type="primary" size="large" block :loading="loading">登录</n-button>
        <small>首次启动默认账号：admin / 123456，登录后必须修改密码。</small>
      </form>
    </section>
  </div>
</template>

