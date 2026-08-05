export const EDITOR_SCHEMA_VERSION = 1 as const
export const MIN_EDITOR_EDGE = 16
export const MAX_EDITOR_EDGE = 8192
export const MAX_EDITOR_PIXELS = 33_554_432

export type EditorFitStrategy = 'cover' | 'contain' | 'free' | 'stretch'
export type EditorAnchor =
  | 'top-left'
  | 'top'
  | 'top-right'
  | 'left'
  | 'center'
  | 'right'
  | 'bottom-left'
  | 'bottom'
  | 'bottom-right'
export type EditorBackground =
  | { type: 'transparent' }
  | { type: 'color'; color: string }
  | { type: 'blurred-image'; blurRadius: number }

export interface EditorCropRect {
  x: number
  y: number
  width: number
  height: number
}

export interface ImageEditorDocumentV1 {
  schemaVersion: 1
  canvas: {
    width: number
    height: number
    background: EditorBackground
  }
  layout: {
    fitStrategy: EditorFitStrategy
    anchor: EditorAnchor
  }
  image: {
    assetId: string
    x: number
    y: number
    scaleX: number
    scaleY: number
    rotation: number
    flipX: boolean
    flipY: boolean
    crop: EditorCropRect
  }
}

export interface EditorAsset {
  id: string
  contentUrl: string
  mimeType: string
  width: number
  height: number
  fileSizeBytes: number
}

export interface ImageEditDocumentResponse {
  id: string
  sourceAssetId: string
  title: string
  schemaVersion: number
  version: number
  document: ImageEditorDocumentV1
  sourceAsset: EditorAsset
  imageAsset: EditorAsset
  createdAt: string
  updatedAt: string
}

const FIT_STRATEGIES = new Set<EditorFitStrategy>(['cover', 'contain', 'free', 'stretch'])
const ANCHORS = new Set<EditorAnchor>([
  'top-left', 'top', 'top-right', 'left', 'center', 'right',
  'bottom-left', 'bottom', 'bottom-right',
])

export function createEditorDocument(
  assetId: string,
  width: number,
  height: number,
): ImageEditorDocumentV1 {
  assertCanvasSize(width, height)
  return {
    schemaVersion: EDITOR_SCHEMA_VERSION,
    canvas: { width, height, background: { type: 'transparent' } },
    layout: { fitStrategy: 'cover', anchor: 'center' },
    image: {
      assetId,
      x: 0,
      y: 0,
      scaleX: 1,
      scaleY: 1,
      rotation: 0,
      flipX: false,
      flipY: false,
      crop: { x: 0, y: 0, width, height },
    },
  }
}

export function cloneEditorDocument(document: ImageEditorDocumentV1): ImageEditorDocumentV1 {
  return JSON.parse(JSON.stringify(document)) as ImageEditorDocumentV1
}

export function assertCanvasSize(width: number, height: number): void {
  if (!Number.isInteger(width) || !Number.isInteger(height)) {
    throw new Error('宽度和高度必须是整数')
  }
  if (width < MIN_EDITOR_EDGE || height < MIN_EDITOR_EDGE) {
    throw new Error(`宽度和高度不能小于 ${MIN_EDITOR_EDGE}px`)
  }
  if (width > MAX_EDITOR_EDGE || height > MAX_EDITOR_EDGE) {
    throw new Error(`宽度和高度不能超过 ${MAX_EDITOR_EDGE}px`)
  }
  if (width * height > MAX_EDITOR_PIXELS) {
    throw new Error('画布总像素不能超过 33,554,432')
  }
}

export function assertCropRect(crop: EditorCropRect, sourceWidth: number, sourceHeight: number): void {
  const values = [crop.x, crop.y, crop.width, crop.height]
  if (values.some((value) => !Number.isInteger(value))) throw new Error('裁剪参数必须是整数像素')
  if (crop.x < 0 || crop.y < 0 || crop.width < MIN_EDITOR_EDGE || crop.height < MIN_EDITOR_EDGE) {
    throw new Error(`裁剪区域不能超出原图，且宽高不能小于 ${MIN_EDITOR_EDGE}px`)
  }
  if (crop.x + crop.width > sourceWidth || crop.y + crop.height > sourceHeight) {
    throw new Error('裁剪区域不能超出原图')
  }
}

export function parseEditorDocument(value: unknown, source: EditorAsset): ImageEditorDocumentV1 {
  if (!isRecord(value) || value.schemaVersion !== EDITOR_SCHEMA_VERSION) {
    throw new Error('不支持的编辑文档版本')
  }
  const canvas = value.canvas
  const layout = value.layout
  const image = value.image
  if (!isRecord(canvas) || !isRecord(layout) || !isRecord(image) || !isRecord(image.crop)) {
    throw new Error('编辑文档结构无效')
  }
  const document = value as unknown as ImageEditorDocumentV1
  assertCanvasSize(document.canvas.width, document.canvas.height)
  assertCropRect(document.image.crop, source.width, source.height)
  if (!FIT_STRATEGIES.has(document.layout.fitStrategy)) throw new Error('未知的图片适配策略')
  if (!ANCHORS.has(document.layout.anchor)) throw new Error('未知的图片对齐方式')
  if (
    !document.image.assetId
    || !Number.isFinite(document.image.x)
    || !Number.isFinite(document.image.y)
    || !Number.isFinite(document.image.scaleX)
    || !Number.isFinite(document.image.scaleY)
    || document.image.scaleX <= 0
    || document.image.scaleY <= 0
    || !Number.isFinite(document.image.rotation)
  ) {
    throw new Error('图片变换参数无效')
  }
  const background = document.canvas.background
  if (
    !isRecord(background)
    || !['transparent', 'color', 'blurred-image'].includes(String(background.type))
    || (background.type === 'color' && typeof background.color !== 'string')
    || (background.type === 'blurred-image'
      && (!Number.isFinite(background.blurRadius) || background.blurRadius < 0 || background.blurRadius > 100))
  ) {
    throw new Error('背景参数无效')
  }
  return cloneEditorDocument(document)
}

export function cropAsCanvas(document: ImageEditorDocumentV1): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  next.canvas.width = Math.round(next.image.crop.width)
  next.canvas.height = Math.round(next.image.crop.height)
  next.layout.fitStrategy = 'free'
  next.image.x = 0
  next.image.y = 0
  next.image.scaleX = 1
  next.image.scaleY = 1
  next.image.rotation = 0
  return next
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
