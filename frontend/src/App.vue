<script setup lang="ts">
import { computed } from 'vue'
import type { GlobalThemeOverrides } from 'naive-ui'
import { darkTheme, NConfigProvider, NDialogProvider, NMessageProvider } from 'naive-ui'
import { useThemeStore } from '@/stores/theme'

const theme = useThemeStore()
theme.initialize()
const naiveTheme = computed(() => (theme.resolved === 'dark' ? darkTheme : null))
const themeOverrides: GlobalThemeOverrides = {
  common: {
    fontFamily: 'Inter, "SF Pro Display", "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
    fontWeight: '400',
    fontWeightStrong: '650',
    borderRadius: '12px',
    borderRadiusSmall: '9px',
    heightMedium: '42px',
    primaryColor: '#7c3aed',
    primaryColorHover: '#8b5cf6',
    primaryColorPressed: '#6d28d9',
    primaryColorSuppl: '#8b5cf6',
  },
  Button: {
    borderRadiusTiny: '8px',
    borderRadiusSmall: '9px',
    borderRadiusMedium: '11px',
    borderRadiusLarge: '12px',
    fontWeight: '600',
  },
  Input: {
    borderRadius: '11px',
    heightMedium: '44px',
  },
  Card: {
    borderRadius: '20px',
    closeBorderRadius: '9px',
    titleFontWeight: '650',
    titleFontSizeMedium: '20px',
    paddingMedium: '22px 24px 24px',
    boxShadow: '0 22px 70px rgba(48, 38, 68, 0.16)',
  },
  Modal: {
    boxShadow: '0 24px 80px rgba(28, 20, 45, 0.24)',
  },
}
</script>

<template>
  <n-config-provider :theme="naiveTheme" :theme-overrides="themeOverrides">
    <n-dialog-provider>
      <n-message-provider>
        <router-view />
      </n-message-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>
