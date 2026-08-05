import { describe, expect, it } from 'vitest'
import { centerCrop, fitCropToRatio, resizeCropWithRatio } from './cropGeometry'

describe('crop geometry', () => {
  it('fits portrait and landscape ratios inside the original source', () => {
    expect(fitCropToRatio(4096, 4096, 16, 9)).toEqual({ x: 0, y: 896, width: 4096, height: 2304 })
    expect(fitCropToRatio(4096, 4096, 9, 16)).toEqual({ x: 896, y: 0, width: 2304, height: 4096 })
  })

  it('resizes only from the edited edge when the ratio is explicitly locked', () => {
    const crop = { x: 10, y: 20, width: 800, height: 600 }
    expect(resizeCropWithRatio(crop, 'width', 960, 16 / 9)).toEqual({ x: 10, y: 20, width: 960, height: 540 })
    expect(resizeCropWithRatio(crop, 'height', 720, 16 / 9)).toEqual({ x: 10, y: 20, width: 1280, height: 720 })
  })

  it('centers an existing crop without changing its dimensions', () => {
    expect(centerCrop({ x: 0, y: 0, width: 261, height: 512 }, 4096, 4096))
      .toEqual({ x: 1918, y: 1792, width: 261, height: 512 })
  })

  it('rejects ratios that would silently violate the minimum crop edge', () => {
    expect(() => fitCropToRatio(1024, 1024, 1000, 1)).toThrow(/至少 16px/)
    expect(() => resizeCropWithRatio({ x: 0, y: 0, width: 16, height: 16 }, 'width', 16, 1000))
      .toThrow(/不能小于 16px/)
  })
})
