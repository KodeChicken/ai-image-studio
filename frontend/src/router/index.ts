import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import DefaultLayout from '@/layouts/DefaultLayout.vue'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', name: 'login', component: () => import('@/views/LoginView.vue') },
    { path: '/editor/:assetId', name: 'image-editor', component: () => import('@/features/image-editor/ImageEditorView.vue') },
    {
      path: '/',
      component: DefaultLayout,
      children: [
        { path: '', redirect: '/studio' },
        { path: 'studio', name: 'studio', component: () => import('@/views/StudioView.vue') },
        { path: 'history', name: 'history', component: () => import('@/views/HistoryView.vue') },
        { path: 'usage', name: 'usage', component: () => import('@/views/UsageView.vue') },
        { path: 'providers', name: 'providers', component: () => import('@/views/ProvidersView.vue') },
        { path: 'admin/users', name: 'users', component: () => import('@/views/UserManagementView.vue'), meta: { admin: true } },
        { path: 'admin/operations', name: 'operations', component: () => import('@/views/OperationsView.vue'), meta: { admin: true } },
        { path: 'settings/storage', name: 'storage', component: () => import('@/views/StorageSettingsView.vue'), meta: { admin: true } },
        { path: 'settings/updates', name: 'updates', component: () => import('@/views/UpdatesView.vue'), meta: { admin: true } },
      ],
    },
  ],
})

router.beforeEach(async (to) => {
  const auth = useAuthStore()
  if (!auth.initialized) await auth.restore()
  if (to.name !== 'login' && !auth.user) return { name: 'login' }
  if (to.name === 'login' && auth.user) return { name: 'studio' }
  if (to.meta.admin && auth.user?.role !== 'admin') return { name: 'studio' }
  return true
})

export default router
