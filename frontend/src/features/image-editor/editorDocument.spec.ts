import { describe, expect, it } from 'vitest'
import {
  assertCanvasSize,
  assertCropRect,
  createEditorDocument,
  cropAsCanvas,
  parseEditorDocument,
  type EditorAsset,
} from './editorDocument'

const asset: EditorAsset = {
  id: 'asset-1', contentUrl: '/asset', mimeType: 'image/png',
  width: 4096, height: 4096, fileSizeBytes: 1,
}

describe('editor document', () => {
  it('keeps an exact 261px crop on a 4K source', () => {
    const document = createEditorDocument(asset.id, asset.width, asset.height)
    document.image.crop.height = 261
    const cropped = cropAsCanvas(parseEditorDocument(document, asset))
    expect(cropped.image.crop.height).toBe(261)
    expect(cropped.canvas.height).toBe(261)
  })

  it('accepts the supported limits and rejects invalid geometry', () => {
    expect(() => assertCanvasSize(8192, 4096)).not.toThrow()
    expect(() => assertCanvasSize(8192, 8192)).toThrow('总像素')
    expect(() => assertCropRect({ x: 0, y: 4000, width: 128, height: 261 }, 4096, 4096)).toThrow('超出原图')
  })
})
