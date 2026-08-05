import {
  cloneEditorDocument,
  type EditorAnchor,
  type EditorFitStrategy,
  type ImageEditorDocumentV1,
} from './editorDocument'

const ANCHOR_FACTORS: Record<EditorAnchor, [number, number]> = {
  'top-left': [0, 0], top: [0.5, 0], 'top-right': [1, 0],
  left: [0, 0.5], center: [0.5, 0.5], right: [1, 0.5],
  'bottom-left': [0, 1], bottom: [0.5, 1], 'bottom-right': [1, 1],
}

export interface FitPlacement {
  x: number
  y: number
  scaleX: number
  scaleY: number
}

export function calculateFitPlacement(
  sourceWidth: number,
  sourceHeight: number,
  canvasWidth: number,
  canvasHeight: number,
  strategy: Exclude<EditorFitStrategy, 'free'>,
  anchor: EditorAnchor,
): FitPlacement {
  const [anchorX, anchorY] = ANCHOR_FACTORS[anchor]
  if (strategy === 'stretch') {
    return { x: 0, y: 0, scaleX: canvasWidth / sourceWidth, scaleY: canvasHeight / sourceHeight }
  }
  const scale = strategy === 'cover'
    ? Math.max(canvasWidth / sourceWidth, canvasHeight / sourceHeight)
    : Math.min(canvasWidth / sourceWidth, canvasHeight / sourceHeight)
  const renderedWidth = sourceWidth * scale
  const renderedHeight = sourceHeight * scale
  return {
    x: (canvasWidth - renderedWidth) * anchorX,
    y: (canvasHeight - renderedHeight) * anchorY,
    scaleX: scale,
    scaleY: scale,
  }
}

export function applyFitStrategy(
  document: ImageEditorDocumentV1,
  strategy: EditorFitStrategy = document.layout.fitStrategy,
  anchor: EditorAnchor = document.layout.anchor,
): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  next.layout.fitStrategy = strategy
  next.layout.anchor = anchor
  if (strategy === 'free') return next
  const placement = calculateFitPlacement(
    next.image.crop.width,
    next.image.crop.height,
    next.canvas.width,
    next.canvas.height,
    strategy,
    anchor,
  )
  Object.assign(next.image, placement)
  next.image.rotation = 0
  return next
}
