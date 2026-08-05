// @vitest-environment jsdom

import { flushPromises, shallowMount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import EditorCanvas from './EditorCanvas.vue'
import ImageEditorView from './ImageEditorView.vue'

const apiMock = vi.hoisted(() => vi.fn())
const streamTaskMock = vi.hoisted(() => vi.fn())
const renderEditorDocumentMock = vi.hoisted(() => vi.fn())
const renderOutpaintInputsMock = vi.hoisted(() => vi.fn())
const routerReplace = vi.hoisted(() => vi.fn())

vi.mock('@/api/client', () => ({
  api: apiMock,
  streamTask: streamTaskMock,
}))

vi.mock('./exporter', async (importOriginal) => ({
  ...await importOriginal<typeof import('./exporter')>(),
  renderEditorDocument: renderEditorDocumentMock,
  renderOutpaintInputs: renderOutpaintInputsMock,
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

function outpaintModel() {
  return {
    id: 'model-1', providerId: 'provider-1', providerType: 'openai-compatible',
    modelKey: 'gpt-image-1', upstreamModelId: 'gpt-image-1', displayName: 'GPT Image 1',
    capabilities: {
      image_edit_capability: {
        supportsImageEdit: true, supportsMask: true, supportsOutpaint: true,
        supportedInputMimeTypes: ['image/png'], supportedOutputSizes: ['1024x1024'],
        maxInputImages: 1, maxDimension: 4096,
      },
    },
    parameterSchema: {
      parameters: {
        size: { type: 'enum', options: ['1024x1024'] },
        n: { type: 'integer' },
        output_format: { type: 'enum', options: ['png'] },
      },
    },
    availabilityStatus: 'verified', discoverySource: 'catalog', capabilitySource: 'catalog',
    lastDiscoveredAt: null, lastVerifiedAt: null, enabled: true,
  }
}

describe('ImageEditorView', () => {
  beforeEach(() => {
    apiMock.mockReset()
    streamTaskMock.mockReset().mockResolvedValue(undefined)
    renderEditorDocumentMock.mockReset().mockResolvedValue(new Blob(['export'], { type: 'image/png' }))
    renderOutpaintInputsMock.mockReset().mockResolvedValue({
      image: new Blob(['source'], { type: 'image/png' }),
      mask: new Blob(['mask'], { type: 'image/png' }),
    })
    apiMock.mockImplementation(async (path: string, options?: RequestInit) => {
      if (path === '/api/v1/image-edit-documents/document-1' && options?.method === 'PUT') {
        const body = JSON.parse(String(options.body))
        return { ...editorResponse(), version: 2, document: body.document }
      }
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

  it('keeps both valid dimensions when they are entered before the next render frame', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const vm = wrapper.vm as unknown as {
      updateCanvasDraft: (edge: 'width' | 'height', value: string) => void
    }

    vm.updateCanvasDraft('width', '1920')
    vm.updateCanvasDraft('height', '1080')
    await new Promise((resolve) => setTimeout(resolve, 30))

    expect(wrapper.findComponent(EditorCanvas).props('document').canvas)
      .toMatchObject({ width: 1920, height: 1080 })
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

  it('locks crop dimensions only when explicitly enabled and applies a true 9:16 crop', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const vm = wrapper.vm as unknown as {
      cropAspectLocked: boolean
      updateCropDraft: (field: 'height', value: string) => void
      applyCropRatio: (width: number, height: number) => void
      restoreFullCrop: () => void
    }

    vm.cropAspectLocked = true
    await flushPromises()
    vm.updateCropDraft('height', '720')
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(wrapper.findComponent(EditorCanvas).props('document').image.crop)
      .toMatchObject({ width: 720, height: 720 })

    vm.cropAspectLocked = false
    await flushPromises()
    vm.updateCropDraft('height', '640')
    await new Promise((resolve) => setTimeout(resolve, 30))
    expect(wrapper.findComponent(EditorCanvas).props('document').image.crop)
      .toMatchObject({ width: 720, height: 640 })

    vm.restoreFullCrop()
    vm.applyCropRatio(9, 16)
    await flushPromises()
    expect(wrapper.findComponent(EditorCanvas).props('document').image.crop)
      .toEqual({ x: 896, y: 0, width: 2304, height: 4096 })
    wrapper.unmount()
  })

  it('rotates around the visual center and keeps invalid angle drafts unchanged', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const vm = wrapper.vm as unknown as { updateRotationDraft: (value: string) => void }

    vm.updateRotationDraft('90')
    await flushPromises()
    const rotated = wrapper.findComponent(EditorCanvas).props('document').image
    expect(rotated.rotation).toBe(90)
    expect(rotated.x).toBeCloseTo(4096)
    expect(rotated.y).toBeCloseTo(0)

    vm.updateRotationDraft('not-an-angle')
    await flushPromises()
    expect(wrapper.findComponent(EditorCanvas).props('document').image.rotation).toBe(90)
    expect((wrapper.find('input[aria-label="图片旋转角度"]').element as HTMLInputElement).value)
      .toBe('not-an-angle')
    wrapper.unmount()
  })

  it('hides AI outpaint when no model declares mask support', async () => {
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    expect(wrapper.find('.editor-toolbar').text()).not.toContain('AI 扩图')
    wrapper.unmount()
  })

  it('uploads a same-task source and mask while preserving the current image on AI failure', async () => {
    let uploadIndex = 0
    let expandRequest: Record<string, unknown> | null = null
    apiMock.mockImplementation(async (path: string, options?: RequestInit) => {
      if (path === '/api/v1/image-edit-documents/document-1') return editorResponse()
      if (path === '/api/v1/models?includeDiscovered=true&imageOnly=true') return [outpaintModel()]
      if (path === '/api/v1/image-assets/uploads') {
        uploadIndex += 1
        return {
          id: `input-${uploadIndex}`, contentUrl: `/input-${uploadIndex}.png`, mimeType: 'image/png',
          width: 1024, height: 1024, fileSizeBytes: 1,
        }
      }
      if (path === '/api/v1/image-edit-documents/document-1/ai-expand') {
        expandRequest = JSON.parse(String(options?.body))
        return { taskId: 'task-1' }
      }
      if (path === '/api/v1/tasks/task-1') return { results: [], errorMessage: 'provider failed' }
      throw new Error(`Unexpected API request: ${path}`)
    })
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const vm = wrapper.vm as unknown as {
      runAiExpand: () => Promise<void>
      tool: string
      aiFailedTaskId: string | null
    }

    await vm.runAiExpand()
    await flushPromises()

    expect(expandRequest).toMatchObject({
      documentVersion: 1,
      sourceAssetId: 'input-1',
      maskAssetId: 'input-2',
      parameters: { size: '1024x1024', n: 1, output_format: 'png' },
    })
    expect(streamTaskMock).toHaveBeenCalledWith('task-1', expect.any(Function), undefined)
    expect(wrapper.findComponent(EditorCanvas).props('document').image.assetId).toBe('asset-1')
    expect(vm.aiFailedTaskId).toBe('task-1')
    wrapper.unmount()
  })

  it('cleans both temporary outpaint inputs when task creation fails', async () => {
    const deleted: string[] = []
    let uploadIndex = 0
    apiMock.mockImplementation(async (path: string, options?: RequestInit) => {
      if (path === '/api/v1/image-edit-documents/document-1') return editorResponse()
      if (path === '/api/v1/models?includeDiscovered=true&imageOnly=true') return [outpaintModel()]
      if (path === '/api/v1/image-assets/uploads') {
        uploadIndex += 1
        return {
          id: `input-${uploadIndex}`, contentUrl: '', mimeType: 'image/png',
          width: 1024, height: 1024, fileSizeBytes: 1,
        }
      }
      if (path.endsWith('/ai-expand')) throw new Error('task creation failed')
      if (options?.method === 'DELETE') {
        deleted.push(path)
        return undefined
      }
      throw new Error(`Unexpected API request: ${path}`)
    })
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()

    await (wrapper.vm as unknown as { runAiExpand: () => Promise<void> }).runAiExpand()

    expect(deleted.sort()).toEqual([
      '/api/v1/image-assets/input-1',
      '/api/v1/image-assets/input-2',
    ])
    expect(wrapper.findComponent(EditorCanvas).props('document').image.assetId).toBe('asset-1')
    wrapper.unmount()
  })

  it('keeps the document unchanged when deterministic export fails', async () => {
    renderEditorDocumentMock.mockRejectedValueOnce(new Error('encoding failed'))
    const wrapper = shallowMount(ImageEditorView)
    await flushPromises()
    const before = JSON.parse(JSON.stringify(wrapper.findComponent(EditorCanvas).props('document')))

    await (wrapper.vm as unknown as { exportImage: () => Promise<void> }).exportImage()

    expect(wrapper.findComponent(EditorCanvas).props('document')).toEqual(before)
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
