import { MIN_EDITOR_EDGE, type EditorCropRect } from './editorDocument'

export function fitCropToRatio(
  sourceWidth: number,
  sourceHeight: number,
  ratioWidth: number,
  ratioHeight: number,
  center?: { x: number; y: number },
): EditorCropRect {
  assertRatio(ratioWidth, ratioHeight)
  const ratio = ratioWidth / ratioHeight
  let width = sourceWidth
  let height = Math.round(width / ratio)
  if (height > sourceHeight) {
    height = sourceHeight
    width = Math.round(height * ratio)
  }
  if (width < MIN_EDITOR_EDGE || height < MIN_EDITOR_EDGE) {
    throw new Error(`该裁剪比例无法在原图内保持至少 ${MIN_EDITOR_EDGE}px 的宽高`)
  }
  const cropCenter = center ?? { x: sourceWidth / 2, y: sourceHeight / 2 }
  return {
    x: clamp(Math.round(cropCenter.x - width / 2), 0, sourceWidth - width),
    y: clamp(Math.round(cropCenter.y - height / 2), 0, sourceHeight - height),
    width,
    height,
  }
}

export function resizeCropWithRatio(
  crop: EditorCropRect,
  edge: 'width' | 'height',
  value: number,
  ratio: number,
): EditorCropRect {
  if (!Number.isFinite(ratio) || ratio <= 0) throw new Error('裁剪比例无效')
  const relatedValue = Math.round(edge === 'width' ? value / ratio : value * ratio)
  if (relatedValue < MIN_EDITOR_EDGE) {
    throw new Error(`锁定比例后的裁剪宽高不能小于 ${MIN_EDITOR_EDGE}px`)
  }
  return edge === 'width'
    ? { ...crop, width: value, height: relatedValue }
    : { ...crop, width: relatedValue, height: value }
}

export function centerCrop(
  crop: EditorCropRect,
  sourceWidth: number,
  sourceHeight: number,
): EditorCropRect {
  return {
    ...crop,
    x: Math.round((sourceWidth - crop.width) / 2),
    y: Math.round((sourceHeight - crop.height) / 2),
  }
}

function assertRatio(width: number, height: number) {
  if (!Number.isFinite(width) || !Number.isFinite(height) || !Number.isFinite(width / height)
    || width <= 0 || height <= 0) {
    throw new Error('裁剪比例必须是正数')
  }
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}
