import { assertCanvasSize, type ImageEditorDocumentV1 } from './editorDocument'

export type EditorExportFormat = 'png' | 'jpeg' | 'webp'

export interface EditorExportOptions {
  format: EditorExportFormat
  quality?: number
}

const MIME_TYPES: Record<EditorExportFormat, string> = {
  png: 'image/png',
  jpeg: 'image/jpeg',
  webp: 'image/webp',
}

export async function renderEditorDocument(
  document: ImageEditorDocumentV1,
  imageUrl: string,
  options: EditorExportOptions,
): Promise<Blob> {
  assertCanvasSize(document.canvas.width, document.canvas.height)
  if (options.format === 'jpeg' && document.canvas.background.type === 'transparent') {
    throw new Error('透明背景不能导出 JPEG，请选择 PNG、WebP 或设置背景色')
  }
  const source = await loadSourceImage(imageUrl)
  const canvas = window.document.createElement('canvas')
  canvas.width = document.canvas.width
  canvas.height = document.canvas.height
  const context = canvas.getContext('2d', { alpha: options.format !== 'jpeg' })
  if (!context) {
    source.close?.()
    throw new Error('当前浏览器无法创建图片导出画布')
  }
  context.imageSmoothingEnabled = true
  context.imageSmoothingQuality = 'high'
  drawBackground(context, source, document)
  drawImageElement(context, source, document)
  source.close?.()
  return canvasToBlob(canvas, MIME_TYPES[options.format], options.quality ?? 0.95)
}

type CanvasSource = CanvasImageSource & { width: number; height: number; close?: () => void }

async function loadSourceImage(imageUrl: string): Promise<CanvasSource> {
  const response = await fetch(imageUrl, { credentials: 'include' })
  if (!response.ok) throw new Error(`读取原图失败（${response.status}）`)
  const blob = await response.blob()
  if ('createImageBitmap' in window) return createImageBitmap(blob)
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const url = URL.createObjectURL(blob)
    const image = new Image()
    image.onload = () => {
      URL.revokeObjectURL(url)
      resolve(image)
    }
    image.onerror = () => {
      URL.revokeObjectURL(url)
      reject(new Error('原图解码失败'))
    }
    image.src = url
  })
}

function drawBackground(
  context: CanvasRenderingContext2D,
  source: CanvasSource,
  document: ImageEditorDocumentV1,
): void {
  const { width, height, background } = document.canvas
  if (background.type === 'color') {
    context.fillStyle = background.color
    context.fillRect(0, 0, width, height)
    return
  }
  if (background.type !== 'blurred-image') return
  const scale = Math.max(width / source.width, height / source.height)
  const destinationWidth = source.width * scale
  const destinationHeight = source.height * scale
  context.save()
  context.filter = `blur(${background.blurRadius}px)`
  context.drawImage(
    source,
    (width - destinationWidth) / 2,
    (height - destinationHeight) / 2,
    destinationWidth,
    destinationHeight,
  )
  context.restore()
}

export function drawImageElement(
  context: CanvasRenderingContext2D,
  source: CanvasImageSource,
  document: ImageEditorDocumentV1,
): void {
  const { image } = document
  const destinationX = image.flipX ? -image.crop.width : 0
  const destinationY = image.flipY ? -image.crop.height : 0
  context.save()
  context.beginPath()
  context.rect(0, 0, document.canvas.width, document.canvas.height)
  context.clip()
  context.translate(image.x, image.y)
  context.rotate(image.rotation * Math.PI / 180)
  context.scale(image.flipX ? -image.scaleX : image.scaleX, image.flipY ? -image.scaleY : image.scaleY)
  context.drawImage(
    source,
    image.crop.x,
    image.crop.y,
    image.crop.width,
    image.crop.height,
    destinationX,
    destinationY,
    image.crop.width,
    image.crop.height,
  )
  context.restore()
}

function canvasToBlob(canvas: HTMLCanvasElement, mimeType: string, quality: number): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => blob ? resolve(blob) : reject(new Error('浏览器未能编码导出图片')),
      mimeType,
      quality,
    )
  })
}
