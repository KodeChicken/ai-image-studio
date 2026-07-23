import { ref } from 'vue'
import { defineStore } from 'pinia'
import { api } from '@/api/client'
import { useThemeStore } from './theme'
import type { CurrentUser, ThemePreference } from '@/types/api'

export const useAuthStore = defineStore('auth', () => {
  const user = ref<CurrentUser | null>(null)
  const initialized = ref(false)

  async function restore() {
    try {
      user.value = await api<CurrentUser>('/api/v1/users/me')
      useThemeStore().set(user.value.themePreference)
    } catch {
      user.value = null
    } finally {
      initialized.value = true
    }
  }

  async function login(username: string, password: string) {
    user.value = await api<CurrentUser>('/api/v1/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    })
    useThemeStore().set(user.value.themePreference)
  }

  async function logout() {
    await api<void>('/api/v1/auth/logout', { method: 'POST' })
    user.value = null
  }

  async function changePassword(currentPassword: string, newPassword: string) {
    await api<void>('/api/v1/users/me/change-password', {
      method: 'POST',
      body: JSON.stringify({ currentPassword, newPassword }),
    })
    if (user.value) user.value.mustChangePassword = false
  }

  async function setTheme(themePreference: ThemePreference) {
    user.value = await api<CurrentUser>('/api/v1/users/me/preferences', {
      method: 'PATCH',
      body: JSON.stringify({ themePreference }),
    })
    useThemeStore().set(themePreference)
  }

  return { user, initialized, restore, login, logout, changePassword, setTheme }
})

