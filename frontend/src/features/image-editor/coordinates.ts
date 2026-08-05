import type { ImageEditorDocumentV1 } from './editorDocument'

export interface Point { x: number; y: number }
export interface ViewportTransform { x: number; y: number; scale: number }

export function documentToViewport(point: Point, viewport: ViewportTransform): Point {
  return { x: viewport.x + point.x * viewport.scale, y: viewport.y + point.y * viewport.scale }
}

export function viewportToDocument(point: Point, viewport: ViewportTransform): Point {
  return { x: (point.x - viewport.x) / viewport.scale, y: (point.y - viewport.y) / viewport.scale }
}

export function sourceToDocument(point: Point, document: ImageEditorDocumentV1): Point {
  const image = document.image
  let x = point.x - image.crop.x
  let y = point.y - image.crop.y
  if (image.flipX) x = image.crop.width - x
  if (image.flipY) y = image.crop.height - y
  x *= image.scaleX
  y *= image.scaleY
  const radians = image.rotation * Math.PI / 180
  return {
    x: image.x + x * Math.cos(radians) - y * Math.sin(radians),
    y: image.y + x * Math.sin(radians) + y * Math.cos(radians),
  }
}

export function documentToSource(point: Point, document: ImageEditorDocumentV1): Point {
  const image = document.image
  const radians = -image.rotation * Math.PI / 180
  const translatedX = point.x - image.x
  const translatedY = point.y - image.y
  let x = (translatedX * Math.cos(radians) - translatedY * Math.sin(radians)) / image.scaleX
  let y = (translatedX * Math.sin(radians) + translatedY * Math.cos(radians)) / image.scaleY
  if (image.flipX) x = image.crop.width - x
  if (image.flipY) y = image.crop.height - y
  return { x: image.crop.x + x, y: image.crop.y + y }
}
