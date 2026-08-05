import { cloneEditorDocument, type ImageEditorDocumentV1 } from './editorDocument'

export function rotateImageAroundCenter(
  document: ImageEditorDocumentV1,
  rotation: number,
): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  const center = imageCenter(document)
  next.layout.fitStrategy = 'free'
  next.image.rotation = normalizeRotation(rotation)
  placeImageCenter(next, center.x, center.y)
  return next
}

export function centerImage(document: ImageEditorDocumentV1): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  next.layout.fitStrategy = 'free'
  placeImageCenter(next, next.canvas.width / 2, next.canvas.height / 2)
  return next
}

export function fitImageToEdge(
  document: ImageEditorDocumentV1,
  edge: 'width' | 'height',
): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  const scale = edge === 'width'
    ? next.canvas.width / next.image.crop.width
    : next.canvas.height / next.image.crop.height
  next.layout.fitStrategy = 'free'
  next.image.rotation = 0
  next.image.scaleX = scale
  next.image.scaleY = scale
  next.image.x = (next.canvas.width - next.image.crop.width * scale) / 2
  next.image.y = (next.canvas.height - next.image.crop.height * scale) / 2
  return next
}

function imageCenter(document: ImageEditorDocumentV1) {
  const radians = document.image.rotation * Math.PI / 180
  const halfWidth = document.image.crop.width * document.image.scaleX / 2
  const halfHeight = document.image.crop.height * document.image.scaleY / 2
  return {
    x: document.image.x + halfWidth * Math.cos(radians) - halfHeight * Math.sin(radians),
    y: document.image.y + halfWidth * Math.sin(radians) + halfHeight * Math.cos(radians),
  }
}

function placeImageCenter(document: ImageEditorDocumentV1, centerX: number, centerY: number) {
  const radians = document.image.rotation * Math.PI / 180
  const halfWidth = document.image.crop.width * document.image.scaleX / 2
  const halfHeight = document.image.crop.height * document.image.scaleY / 2
  document.image.x = centerX - halfWidth * Math.cos(radians) + halfHeight * Math.sin(radians)
  document.image.y = centerY - halfWidth * Math.sin(radians) - halfHeight * Math.cos(radians)
}

function normalizeRotation(rotation: number) {
  return ((rotation % 360) + 360) % 360
}
