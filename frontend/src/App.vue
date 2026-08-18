<script setup lang="ts">
import { computed } from 'vue'
import type { GlobalThemeOverrides } from 'naive-ui'
import { darkTheme, NConfigProvider, NDialogProvider, NMessageProvider } from 'naive-ui'
import { imageEditorRouteLoading } from '@/router'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'

const auth = useAuthStore()
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
        <div
          v-if="imageEditorRouteLoading"
          class="editor-route-loading"
          role="status"
          aria-live="polite"
        ><i aria-hidden="true"></i><span>正在打开图片编辑器…</span></div>
        <router-view v-slot="{ Component, route }">
          <keep-alive include="DefaultLayout" :max="1">
            <component
              :is="Component"
              :key="route.name === 'image-editor' ? route.fullPath : auth.user?.id ?? route.fullPath"
            />
          </keep-alive>
        </router-view>
      </n-message-provider>
    </n-dialog-provider>
  </n-config-provider>
</template>

<style scoped>
.editor-route-loading { position: fixed; z-index: 10000; top: 14px; left: 50%; display: flex; align-items: center; gap: 8px; padding: 9px 13px; border: 1px solid rgba(255,255,255,.14); border-radius: 999px; color: #f5f3ff; background: rgba(30,31,37,.94); box-shadow: 0 10px 32px rgba(0,0,0,.3); font-size: 12px; transform: translateX(-50%); }
.editor-route-loading i { width: 13px; height: 13px; border: 2px solid rgba(196,181,253,.35); border-top-color: #a78bfa; border-radius: 50%; animation: editor-route-spin .7s linear infinite; }
@keyframes editor-route-spin { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .editor-route-loading i { animation: none; } }
</style>
