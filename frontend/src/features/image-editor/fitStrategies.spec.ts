import { describe, expect, it } from 'vitest'
import { calculateFitPlacement } from './fitStrategies'

describe('fit strategies', () => {
  it('covers a 1920x1080 canvas with a square source without stretching', () => {
    expect(calculateFitPlacement(1024, 1024, 1920, 1080, 'cover', 'center')).toEqual({
      x: 0, y: -420, scaleX: 1.875, scaleY: 1.875,
    })
  })

  it('contains a landscape source in a 1080x1440 canvas', () => {
    expect(calculateFitPlacement(1920, 1080, 1080, 1440, 'contain', 'center')).toEqual({
      x: 0, y: 416.25, scaleX: 0.5625, scaleY: 0.5625,
    })
  })

  it('stretches only when explicitly selected', () => {
    expect(calculateFitPlacement(1000, 1000, 1600, 900, 'stretch', 'center')).toEqual({
      x: 0, y: 0, scaleX: 1.6, scaleY: 0.9,
    })
  })
})
