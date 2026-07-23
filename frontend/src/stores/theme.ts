import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import type { ThemePreference } from '@/types/api'

export const useThemeStore = defineStore('theme', () => {
  const preference = ref<ThemePreference>('system')
  const systemDark = ref(false)
  const resolved = computed<'light' | 'dark'>(() =>
    preference.value === 'system' ? (systemDark.value ? 'dark' : 'light') : preference.value,
  )

  function initialize() {
    const saved = localStorage.getItem('theme-preference') as ThemePreference | null
    if (saved && ['light', 'dark', 'system'].includes(saved)) preference.value = saved
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    systemDark.value = media.matches
    media.addEventListener('change', (event) => (systemDark.value = event.matches))
    apply()
    watch([preference, systemDark], apply)
  }

  function set(value: ThemePreference) {
    preference.value = value
    localStorage.setItem('theme-preference', value)
  }

  function toggle() {
    set(resolved.value === 'dark' ? 'light' : 'dark')
  }

  function apply() {
    document.documentElement.dataset.theme = resolved.value
    document.documentElement.style.colorScheme = resolved.value
  }

  return { preference, resolved, initialize, set, toggle }
})

