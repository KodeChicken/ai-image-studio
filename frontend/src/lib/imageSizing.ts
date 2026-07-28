import type { ParameterDefinition } from '@/types/api'

export interface ImageSize {
  width: number
  height: number
}

export type FixedImageEdge = 'width' | 'height'

export function parseImageSize(value: unknown): ImageSize | null {
  if (typeof value !== 'string') return null
  const match = /^(\d+)x(\d+)$/.exec(value.trim().toLowerCase())
  if (!match) return null
  const width = Number(match[1])
  const height = Number(match[2])
  return Number.isSafeInteger(width) && Number.isSafeInteger(height) && width > 0 && height > 0
    ? { width, height }
    : null
}

export function imageAspectRatio(size: ImageSize): string {
  const divisor = greatestCommonDivisor(size.width, size.height)
  return `${size.width / divisor}:${size.height / divisor}`
}

export function aspectRatioMatches(size: ImageSize, aspectRatio: string): boolean {
  const parsed = parseAspectRatio(aspectRatio)
  return parsed !== null && size.width * parsed.height === size.height * parsed.width
}

export function validCustomImageSize(
  size: ImageSize,
  definition: ParameterDefinition,
): boolean {
  const constraints = definition.constraints ?? {}
  const edgeMultiple = Math.max(1, constraints.edgeMultiple ?? 1)
  const maxEdge = constraints.maxEdge ?? Number.MAX_SAFE_INTEGER
  const minPixels = constraints.minPixels ?? 1
  const maxPixels = constraints.maxPixels ?? Number.MAX_SAFE_INTEGER
  const maxAspectRatio = Math.max(1, constraints.maxAspectRatio ?? Number.MAX_SAFE_INTEGER)
  const pixels = size.width * size.height
  const longEdge = Math.max(size.width, size.height)
  const shortEdge = Math.min(size.width, size.height)
  return Number.isSafeInteger(pixels)
    && size.width % edgeMultiple === 0
    && size.height % edgeMultiple === 0
    && longEdge <= maxEdge
    && pixels >= minPixels
    && pixels <= maxPixels
    && longEdge <= shortEdge * maxAspectRatio
}

export function dependentEdgeOptions(
  fixedValue: number,
  fixedEdge: FixedImageEdge,
  definition: ParameterDefinition,
): number[] {
  const constraints = definition.constraints ?? {}
  const edgeMultiple = Math.max(1, constraints.edgeMultiple ?? 1)
  const maxEdge = constraints.maxEdge ?? 4096
  if (!Number.isInteger(fixedValue) || fixedValue <= 0 || fixedValue > maxEdge || fixedValue % edgeMultiple !== 0) {
    return []
  }
  const options: number[] = []
  for (let value = edgeMultiple; value <= maxEdge; value += edgeMultiple) {
    const size = fixedEdge === 'width'
      ? { width: fixedValue, height: value }
      : { width: value, height: fixedValue }
    if (validCustomImageSize(size, definition)) options.push(value)
  }
  return options
}

export function closestNumber(options: number[], target: number): number | null {
  return options.reduce<number | null>((closest, value) => (
    closest === null || Math.abs(value - target) < Math.abs(closest - target) ? value : closest
  ), null)
}

function parseAspectRatio(value: string): ImageSize | null {
  const match = /^(\d+):(\d+)$/.exec(value)
  if (!match) return null
  const width = Number(match[1])
  const height = Number(match[2])
  return width > 0 && height > 0 ? { width, height } : null
}

function greatestCommonDivisor(left: number, right: number): number {
  let a = left
  let b = right
  while (b !== 0) [a, b] = [b, a % b]
  return a
}
