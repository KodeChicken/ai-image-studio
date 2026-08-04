// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import ImageCropModal from './ImageCropModal.vue'

interface CropData {
  x: number
  y: number
  width: number
  height: number
}

interface CropperOptions {
  viewMode: number
  autoCropArea: number
  ready: () => void
  crop: (event: { detail: CropData }) => void
}

interface MockCropper {
  options: CropperOptions
  data: CropData
  setData: ReturnType<typeof vi.fn>
  getCroppedCanvas: ReturnType<typeof vi.fn>
}

const cropperHarness = vi.hoisted(() => ({ instances: [] as MockCropper[] }))
const messageHarness = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }))

vi.mock('cropperjs', () => ({
  default: class {
    options: CropperOptions
    data: CropData = { x: 0, y: 0, width: 840, height: 840 }
    setData = vi.fn((next: Partial<CropData>) => {
      this.data = { ...this.data, ...next }
      this.options.crop({ detail: { ...this.data } })
      return this
    })
    getCroppedCanvas = vi.fn(() => {
      const canvas = document.createElement('canvas')
      canvas.width = Math.round(this.data.width)
      canvas.height = Math.round(this.data.height)
      canvas.toBlob = (callback) => callback(new Blob(['crop'], { type: 'image/png' }))
      return canvas
    })

    constructor(_image: HTMLImageElement, options: CropperOptions) {
      this.options = options
      cropperHarness.instances.push(this)
    }

    getImageData() {
      return { naturalWidth: 1024, naturalHeight: 1024, width: 840, height: 840 }
    }

    getData() {
      return { ...this.data }
    }

    destroy() {}
  },
}))

vi.mock('naive-ui', () => ({
  NModal: {
    props: ['show'],
    template: '<div v-if="show"><slot /></div>',
  },
  NButton: {
    props: ['disabled', 'loading'],
    emits: ['click'],
    template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
  },
  useMessage: () => messageHarness,
}))

const image = {
  id: 'asset-1',
  contentUrl: '/api/v1/image-assets/asset-1/content',
  label: '测试图片',
  metadata: '960 × 128 · image/png',
  mimeType: 'image/png',
  width: 960,
  height: 128,
}

async function mountCropper() {
  const wrapper = mount(ImageCropModal, {
    props: { show: true, image, initialMode: 'preview' },
  })
  const cropButton = wrapper.findAll('button').find((button) => button.text() === '裁剪缩放')
  expect(cropButton).toBeDefined()
  await cropButton!.trigger('click')
  await wrapper.get('.image-crop-canvas img').trigger('load')
  const instance = cropperHarness.instances[cropperHarness.instances.length - 1]!
  instance.options.ready()
  await flushPromises()
  return { wrapper, instance }
}

describe('ImageCropModal', () => {
  beforeEach(() => {
    cropperHarness.instances.length = 0
    messageHarness.success.mockReset()
    messageHarness.error.mockReset()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('starts with the complete original image selected', async () => {
    const { wrapper, instance } = await mountCropper()

    expect(instance.options.viewMode).toBe(1)
    expect(instance.options.autoCropArea).toBe(1)
    expect(instance.setData).toHaveBeenCalledWith({ x: 0, y: 0, width: 1024, height: 1024 })
    expect(wrapper.get<HTMLInputElement>('input[aria-label="裁剪宽度"]').element.value).toBe('1024')
    expect(wrapper.get<HTMLInputElement>('input[aria-label="裁剪高度"]').element.value).toBe('1024')

    wrapper.unmount()
  })

  it('keeps crop dimensions synchronized in original-image pixels', async () => {
    const { wrapper, instance } = await mountCropper()
    instance.options.crop({ detail: { x: 100, y: 120, width: 640, height: 480 } })
    await flushPromises()

    expect(wrapper.get<HTMLInputElement>('input[aria-label="裁剪宽度"]').element.value).toBe('640')
    expect(wrapper.get<HTMLInputElement>('input[aria-label="裁剪高度"]').element.value).toBe('480')

    instance.data = { x: 0, y: 0, width: 1024, height: 1024 }
    instance.setData.mockClear()
    await wrapper.get('input[aria-label="裁剪宽度"]').setValue('960')
    expect(instance.setData).toHaveBeenLastCalledWith({ x: 32, width: 960 })
    await wrapper.get('input[aria-label="裁剪高度"]').setValue('128')
    expect(instance.setData).toHaveBeenLastCalledWith({ y: 448, height: 128 })
    expect(wrapper.get<HTMLInputElement>('input[aria-label="裁剪宽度"]').element.value).toBe('960')

    wrapper.unmount()
  })

  it('exports the original crop without passing resize dimensions', async () => {
    const anchorClick = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined)
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn(() => 'blob:crop'),
      revokeObjectURL: vi.fn(),
    })
    const { wrapper, instance } = await mountCropper()
    await wrapper.get('input[aria-label="裁剪宽度"]').setValue('960')
    await wrapper.get('input[aria-label="裁剪高度"]').setValue('128')

    const exportButton = wrapper.findAll('button').find((button) => button.text() === '导出成品')
    await exportButton!.trigger('click')
    await flushPromises()

    const exportOptions = instance.getCroppedCanvas.mock.calls[0]![0] as Record<string, unknown>
    expect(exportOptions).not.toHaveProperty('width')
    expect(exportOptions).not.toHaveProperty('height')
    expect(messageHarness.success).toHaveBeenCalledWith('已导出 960 × 128')
    expect(anchorClick).toHaveBeenCalledOnce()

    wrapper.unmount()
  })
})
