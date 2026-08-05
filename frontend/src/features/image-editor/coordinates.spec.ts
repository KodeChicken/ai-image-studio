import { describe, expect, it } from 'vitest'
import { createEditorDocument } from './editorDocument'
import { documentToSource, documentToViewport, sourceToDocument, viewportToDocument } from './coordinates'

describe('editor coordinates', () => {
  it('keeps viewport transforms separate from document pixels', () => {
    const viewport = { x: 120, y: 80, scale: 0.25 }
    const point = { x: 1024, y: 576 }
    expect(viewportToDocument(documentToViewport(point, viewport), viewport)).toEqual(point)
  })

  it('round-trips rotated, scaled and flipped source points', () => {
    const document = createEditorDocument('asset', 1024, 1024)
    Object.assign(document.image, { x: 40, y: 90, scaleX: 1.5, scaleY: 0.8, rotation: 27, flipX: true })
    const source = { x: 261, y: 700 }
    const result = documentToSource(sourceToDocument(source, document), document)
    expect(result.x).toBeCloseTo(source.x, 8)
    expect(result.y).toBeCloseTo(source.y, 8)
  })
})
