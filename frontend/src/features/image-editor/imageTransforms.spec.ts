import { describe, expect, it } from 'vitest'
import { createEditorDocument } from './editorDocument'
import { centerImage, fitImageToEdge, rotateImageAroundCenter } from './imageTransforms'

describe('image transforms', () => {
  it('rotates around the current visual center', () => {
    const document = createEditorDocument('asset', 1000, 500)
    const rotated = rotateImageAroundCenter(document, 90)
    expect(rotated.image.x).toBeCloseTo(750)
    expect(rotated.image.y).toBeCloseTo(-250)
    expect(rotated.image.rotation).toBe(90)
  })

  it('centers a rotated image and fits either edge without stretching', () => {
    const document = createEditorDocument('asset', 1000, 500)
    document.canvas.width = 800
    document.canvas.height = 800
    document.image.rotation = 90
    const centered = centerImage(document)
    expect(centered.image.x).toBeCloseTo(650)
    expect(centered.image.y).toBeCloseTo(-100)

    const fitted = fitImageToEdge(document, 'width')
    expect(fitted.image.scaleX).toBe(0.8)
    expect(fitted.image.scaleY).toBe(0.8)
    expect(fitted.image.y).toBe(200)
  })
})
