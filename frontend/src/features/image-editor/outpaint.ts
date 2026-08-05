import type { ImageModel, ParameterDefinition } from '@/types/api'
import { parseImageSize, validCustomImageSize, type ImageSize } from '@/lib/imageSizing'
import { cloneEditorDocument, type ImageEditorDocumentV1 } from './editorDocument'

export interface OutpaintPreparation {
  document: ImageEditorDocumentV1
  parameters: Record<string, unknown>
  size: ImageSize
}

export function prepareOutpaint(
  document: ImageEditorDocumentV1,
  model: ImageModel,
): OutpaintPreparation {
  const size = chooseOutpaintSize(model, {
    width: document.canvas.width,
    height: document.canvas.height,
  })
  return {
    document: mapDocumentToProviderCanvas(document, size),
    parameters: buildOutpaintParameters(model, size),
    size,
  }
}

export function modelHasExplicitOutpaintSize(model: ImageModel): boolean {
  const sizeDefinition = model.parameterSchema.parameters?.size
  if (!sizeDefinition) return false
  if (sizeDefinition.allow_custom) return true
  const capability = model.capabilities.image_edit_capability as { supportedOutputSizes?: unknown } | undefined
  return [
    ...(Array.isArray(capability?.supportedOutputSizes) ? capability.supportedOutputSizes : []),
    ...(sizeDefinition.options ?? []),
  ].some((value) => parseImageSize(value) !== null)
}

export function chooseOutpaintSize(model: ImageModel, target: ImageSize): ImageSize {
  const sizeDefinition = model.parameterSchema.parameters?.size
  if (sizeDefinition?.allow_custom && validCustomImageSize(target, sizeDefinition)) return target

  const capability = model.capabilities.image_edit_capability as { supportedOutputSizes?: unknown } | undefined
  const declared = capability?.supportedOutputSizes
  const candidates = [
    ...(Array.isArray(declared) ? declared : []),
    ...(sizeDefinition?.options ?? []),
  ]
    .map(parseImageSize)
    .filter((size): size is ImageSize => size !== null)
    .filter((size, index, values) => values.findIndex((item) => item.width === size.width && item.height === size.height) === index)

  const closest = candidates.reduce<ImageSize | null>((best, candidate) => {
    if (!best) return candidate
    return sizeScore(candidate, target) < sizeScore(best, target) ? candidate : best
  }, null)
  if (!closest) throw new Error('该模型没有可用于扩图的明确输出尺寸')
  return closest
}

export function mapDocumentToProviderCanvas(
  document: ImageEditorDocumentV1,
  size: ImageSize,
): ImageEditorDocumentV1 {
  const next = cloneEditorDocument(document)
  const scale = Math.min(size.width / next.canvas.width, size.height / next.canvas.height)
  const offsetX = (size.width - next.canvas.width * scale) / 2
  const offsetY = (size.height - next.canvas.height * scale) / 2
  next.canvas.width = size.width
  next.canvas.height = size.height
  next.canvas.background = { type: 'transparent' }
  next.image.x = offsetX + next.image.x * scale
  next.image.y = offsetY + next.image.y * scale
  next.image.scaleX *= scale
  next.image.scaleY *= scale
  return next
}

function buildOutpaintParameters(model: ImageModel, size: ImageSize) {
  const definitions = model.parameterSchema.parameters ?? {}
  if (!definitions.size) throw new Error('该模型没有声明可传递的扩图尺寸参数')
  const parameters: Record<string, unknown> = {}
  addParameter(parameters, definitions, 'size', `${size.width}x${size.height}`)
  addParameter(parameters, definitions, 'n', 1)
  addParameter(parameters, definitions, 'output_format', 'png')
  if (definitions.background?.options?.includes('opaque')) {
    addParameter(parameters, definitions, 'background', 'opaque')
  }
  return parameters
}

function addParameter(
  parameters: Record<string, unknown>,
  definitions: Record<string, ParameterDefinition>,
  name: string,
  value: unknown,
) {
  if (definitions[name]) parameters[name] = value
}

function sizeScore(candidate: ImageSize, target: ImageSize) {
  const ratioDistance = Math.abs(Math.log((candidate.width / candidate.height) / (target.width / target.height)))
  const areaDistance = Math.abs(Math.log((candidate.width * candidate.height) / (target.width * target.height)))
  return ratioDistance * 10 + areaDistance
}
