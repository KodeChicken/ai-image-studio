// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createEditorDocument } from './editorDocument'
import { renderEditorDocument, renderOutpaintInputs, type EditorExportFormat } from './exporter'

describe('deterministic editor export', () => {
  const drawImage = vi.fn()
  const toBlob = vi.fn()
  const encodedSizes: Array<{ width: number; height: number; mimeType: string }> = []
  const contexts: Array<CanvasRenderingContext2D> = []

  beforeEach(() => {
    drawImage.mockReset()
    toBlob.mockReset()
    encodedSizes.length = 0
    contexts.length = 0
    vi.stubGlobal('fetch', vi.fn(async () => new Response(new Blob(['source'], { type: 'image/png' }))))
    vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 4096, height: 4096, close: vi.fn() })))
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => {
      const context = {
        imageSmoothingEnabled: false,
        imageSmoothingQuality: 'low',
        fillStyle: '',
        globalCompositeOperation: 'source-over',
        fillRect: vi.fn(),
        drawImage,
        save: vi.fn(),
        restore: vi.fn(),
        beginPath: vi.fn(),
        rect: vi.fn(),
        clip: vi.fn(),
        translate: vi.fn(),
        rotate: vi.fn(),
        scale: vi.fn(),
        filter: '',
      } as unknown as CanvasRenderingContext2D
      contexts.push(context)
      return context
    })
    toBlob.mockImplementation(function (this: HTMLCanvasElement, callback: BlobCallback, mimeType: string) {
      encodedSizes.push({ width: this.width, height: this.height, mimeType })
      callback(new Blob(['encoded'], { type: mimeType }))
    })
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(toBlob)
  })

  afterEach(() => vi.restoreAllMocks())

  it.each([
    ['png', 'image/png'],
    ['jpeg', 'image/jpeg'],
    ['webp', 'image/webp'],
  ] as Array<[EditorExportFormat, string]>)('encodes exact 1920x1080 %s output from the original asset', async (format, mimeType) => {
    const document = createEditorDocument('asset', 4096, 4096)
    document.canvas.width = 1920
    document.canvas.height = 1080
    if (format === 'jpeg') document.canvas.background = { type: 'color', color: '#ffffff' }

    const blob = await renderEditorDocument(document, '/api/v1/image-assets/asset/content', { format })
    expect(fetch).toHaveBeenCalledWith('/api/v1/image-assets/asset/content', { credentials: 'include' })
    expect(encodedSizes[0]).toEqual({ width: 1920, height: 1080, mimeType })
    expect(blob.type).toBe(mimeType)
    expect(drawImage).toHaveBeenCalled()
  })

  it('renders same-size PNG source and alpha mask for outpaint', async () => {
    const document = createEditorDocument('asset', 1024, 1024)
    document.canvas.width = 1536
    document.canvas.height = 1024

    const inputs = await renderOutpaintInputs(document, '/api/v1/image-assets/asset/content')

    expect(encodedSizes).toEqual([
      { width: 1536, height: 1024, mimeType: 'image/png' },
      { width: 1536, height: 1024, mimeType: 'image/png' },
    ])
    expect(inputs.image.type).toBe('image/png')
    expect(inputs.mask.type).toBe('image/png')
    expect(contexts[1]!.globalCompositeOperation).toBe('source-in')
    expect(contexts[1]!.fillRect).toHaveBeenCalledWith(0, 0, 1536, 1024)
  })
})
