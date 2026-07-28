import { describe, expect, it } from 'vitest'
import {
  aspectRatioMatches,
  dependentEdgeOptions,
  imageAspectRatio,
  parseImageSize,
  validCustomImageSize,
} from './imageSizing'
import type { ParameterDefinition } from '@/types/api'

const gptImage2Size: ParameterDefinition = {
  type: 'enum',
  allow_custom: true,
  constraints: {
    edgeMultiple: 16,
    maxEdge: 3840,
    minPixels: 655360,
    maxPixels: 8294400,
    maxAspectRatio: 3,
  },
}

describe('image sizing', () => {
  it('derives and compares exact aspect ratios', () => {
    const portrait = parseImageSize('2160x3840')!
    expect(imageAspectRatio(portrait)).toBe('9:16')
    expect(aspectRatioMatches(portrait, '9:16')).toBe(true)
    expect(aspectRatioMatches(portrait, '16:9')).toBe(false)
  })

  it('enforces GPT Image 2 custom-size constraints', () => {
    expect(validCustomImageSize({ width: 1536, height: 864 }, gptImage2Size)).toBe(true)
    expect(validCustomImageSize({ width: 960, height: 128 }, gptImage2Size)).toBe(false)
    expect(validCustomImageSize({ width: 1537, height: 864 }, gptImage2Size)).toBe(false)
  })

  it('only offers valid dependent edges for a fixed width', () => {
    const heights = dependentEdgeOptions(960, 'width', gptImage2Size)
    expect(heights).toContain(688)
    expect(heights).not.toContain(128)
    expect(heights.every((height) => validCustomImageSize({ width: 960, height }, gptImage2Size))).toBe(true)
  })
})
