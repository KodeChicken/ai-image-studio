// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createEditorDocument } from './editorDocument'
import { renderEditorDocument, type EditorExportFormat } from './exporter'

describe('deterministic editor export', () => {
  const drawImage = vi.fn()
  const toBlob = vi.fn()

  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(new Blob(['source'], { type: 'image/png' }))))
    vi.stubGlobal('createImageBitmap', vi.fn(async () => ({ width: 4096, height: 4096, close: vi.fn() })))
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      imageSmoothingEnabled: false,
      imageSmoothingQuality: 'low',
      fillStyle: '',
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
    } as unknown as CanvasRenderingContext2D)
    toBlob.mockImplementation((callback: BlobCallback, mimeType: string) => {
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
    const canvas = toBlob.mock.instances[0] as HTMLCanvasElement

    expect(fetch).toHaveBeenCalledWith('/api/v1/image-assets/asset/content', { credentials: 'include' })
    expect(canvas.width).toBe(1920)
    expect(canvas.height).toBe(1080)
    expect(blob.type).toBe(mimeType)
    expect(drawImage).toHaveBeenCalled()
  })
})
