// @vitest-environment jsdom

import { flushPromises, shallowMount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import EditorCanvas from './EditorCanvas.vue'
import ImageEditorView from './ImageEditorView.vue'

const apiMock = vi.hoisted(() => vi.fn())
const routerReplace = vi.hoisted(() => vi.fn())

vi.mock('@/api/client', () => ({
  api: apiMock,
  streamTask: vi.fn(),
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: { assetId: 'asset-1' }, query: { documentId: 'document-1' } }),
  useRouter: () => ({ back: vi.fn(), replace: routerReplace }),
  onBeforeRouteLeave: vi.fn(),
}))

vi.mock('naive-ui', async () => {
  const actual = await vi.importActual<typeof import('naive-ui')>('naive-ui')
  return {
    ...actual,
    useMessage: () => ({ error: vi.fn(), success: vi.fn(), warning: vi.fn() }),
  }
})

const timestamp = '2026-08-05T00:00:00.000Z'

function editorResponse() {
  return {
    id: 'document-1',
    sourceAssetId: 'asset-1',
    title: '图片成品',
    schemaVersion: 1,
    version: 1,
    document: {
      schemaVersion: 1,
      canvas: { width: 4096, height: 4096, background: { type: 'transparent' } },
      layout: { fitStrategy: 'cover', anchor: 'center' },
      image: {
        assetId: 'asset-1', x: 0, y: 0, scaleX: 1, scaleY: 1,
        rotation: 0, flipX: false, flipY: false,
        crop: { x: 0, y: 0, width: 4096, height: 4096 },
      },
    },
    sourceAsset: {
      id: 'asset-1', contentUrl: '/api/v1/image-assets/asset-1/content',
      mimeType: 'image/png', width: 4096, height: 4096, fileSizeBytes: 1,
    },
    imageAsset: {
      id: 'asset-1', contentUrl: '/api/v1/image-assets/asset-1/content',
      mimeType: 'image/png', width: 4096, height: 4096, fileSizeBytes: 1,
    },
    createdAt: timestamp,
    updatedAt: timestamp,
  }
}

describe('ImageEditorView', () => {
  beforeEach(() => {
    apiMock.mockReset()
    apiMock.mockImplementation(async (path: string) => {
      if (path === '/api/v1/image-edit-documents/document-1') return editorResponse()
      if (path === '/api/v1/models?includeDiscovered=true&imageOnly=true') return []
      throw new Error(`Unexpected API request: ${path}`)
    })
  })

  afterEach(() => vi.restoreAllMocks())

  it('keeps width and height independent and preserves invalid drafts', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const fields = wrapper.findAll<HTMLInputElement>('.dimension-fields input')
    expect(fields).toHaveLength(2)

    await fields[0]!.setValue('261')
    await new Promise((resolve) => setTimeout(resolve, 30))

    const canvas = wrapper.findComponent(EditorCanvas)
    expect(canvas.props('document').canvas).toMatchObject({ width: 261, height: 4096 })
    expect(fields[1]!.element.value).toBe('4096')

    await fields[0]!.setValue('99999')
    await fields[0]!.trigger('blur')
    expect(fields[0]!.element.value).toBe('99999')
    expect(canvas.props('document').canvas.width).toBe(261)
    wrapper.unmount()
  })

  it('applies an exact 261px crop to a 4K source without preview rounding', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    await wrapper.findAll('.editor-toolbar button')[1]!.trigger('click')
    const cropFields = wrapper.findAll<HTMLInputElement>('.four-fields input')

    await cropFields[3]!.setValue('261')
    await new Promise((resolve) => setTimeout(resolve, 30))

    const document = wrapper.findComponent(EditorCanvas).props('document')
    expect(document.image.crop.height).toBe(261)
    expect(document.canvas.height).toBe(261)
    wrapper.unmount()
  })

  it('restores the original source asset after reopening a derived document', async () => {
    const response = editorResponse()
    response.document.image.assetId = 'asset-2'
    response.document.image.crop = { x: 0, y: 0, width: 2048, height: 2048 }
    response.imageAsset = {
      id: 'asset-2', contentUrl: '/api/v1/image-assets/asset-2/content',
      mimeType: 'image/png', width: 2048, height: 2048, fileSizeBytes: 1,
    }
    apiMock.mockImplementation(async (path: string) => {
      if (path === '/api/v1/image-edit-documents/document-1') return response
      if (path === '/api/v1/models?includeDiscovered=true&imageOnly=true') return []
      throw new Error(`Unexpected API request: ${path}`)
    })

    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    ;(wrapper.vm as unknown as { resetDocument: () => void }).resetDocument()
    await flushPromises()

    const canvas = wrapper.findComponent(EditorCanvas)
    expect(canvas.props('document').image.assetId).toBe('asset-1')
    expect(canvas.props('imageUrl')).toBe('/api/v1/image-assets/asset-1/content')
    expect(canvas.props('document').canvas).toMatchObject({ width: 4096, height: 4096 })
    wrapper.unmount()
  })
})
