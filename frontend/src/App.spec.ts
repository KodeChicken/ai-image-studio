// @vitest-environment jsdom
/* eslint-disable vue/one-component-per-file */

import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { describe, expect, it, vi } from 'vitest'

import App from './App.vue'

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({ user: { id: 'user-1' } }),
}))

vi.mock('@/stores/theme', () => ({
  useThemeStore: () => ({ initialize: vi.fn(), resolved: 'light' }),
}))

describe('App', () => {
  it('keeps the authenticated layout alive while the editor is open', async () => {
    let layoutMounts = 0
    const DefaultLayout = defineComponent({
      name: 'DefaultLayout',
      setup() {
        layoutMounts += 1
        return () => h('div', { id: 'studio-view' }, 'Studio')
      },
    })
    const ImageEditorView = defineComponent({
      name: 'ImageEditorView',
      setup: () => () => h('div', { id: 'editor-view' }, 'Editor'),
    })
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: '/studio', name: 'studio', component: DefaultLayout },
        { path: '/editor/:assetId', name: 'image-editor', component: ImageEditorView },
      ],
    })
    await router.push('/studio')
    await router.isReady()
    const wrapper = mount(App, { global: { plugins: [router] } })
    await flushPromises()

    await router.push('/editor/asset-1')
    await flushPromises()
    expect(wrapper.find('#editor-view').exists()).toBe(true)
    await router.push('/studio')
    await flushPromises()

    expect(wrapper.find('#studio-view').exists()).toBe(true)
    expect(layoutMounts).toBe(1)
    wrapper.unmount()
  })
})
