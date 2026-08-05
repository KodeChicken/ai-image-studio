<script setup lang="ts">
import Konva from 'konva'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { cloneEditorDocument, type ImageEditorDocumentV1 } from './editorDocument'

export type EditorTool = 'select' | 'crop' | 'canvas' | 'background' | 'ai'

const props = defineProps<{
  document: ImageEditorDocumentV1
  imageUrl: string
  sourceWidth: number
  sourceHeight: number
  tool: EditorTool
}>()

const emit = defineEmits<{
  commit: [document: ImageEditorDocumentV1]
  zoom: [percent: number]
}>()

const container = ref<HTMLDivElement | null>(null)
const loadingImage = ref(true)
let stage: Konva.Stage | null = null
let layer: Konva.Layer | null = null
let sourceImage: HTMLImageElement | null = null
let observer: ResizeObserver | null = null
let zoomMultiplier = 1
let panX = 0
let panY = 0
let panning = false
let panPointer = { x: 0, y: 0 }
let renderFrame = 0

onMounted(() => {
  if (!container.value) return
  stage = new Konva.Stage({ container: container.value, width: 1, height: 1 })
  layer = new Konva.Layer()
  stage.add(layer)
  stage.on('wheel', handleWheel)
  stage.on('pointerdown', beginPan)
  stage.on('pointermove', movePan)
  stage.on('pointerup pointercancel', endPan)
  observer = new ResizeObserver(resizeStage)
  observer.observe(container.value)
  resizeStage()
  loadImage()
})

onBeforeUnmount(() => {
  cancelAnimationFrame(renderFrame)
  observer?.disconnect()
  sourceImage = null
  stage?.destroy()
})

watch(() => props.imageUrl, loadImage)
watch(
  () => [props.document, props.tool, props.sourceWidth, props.sourceHeight],
  scheduleRender,
  { deep: true },
)

function loadImage() {
  loadingImage.value = true
  sourceImage = null
  const image = new Image()
  image.onload = () => {
    sourceImage = image
    loadingImage.value = false
    scheduleRender()
  }
  image.onerror = () => {
    loadingImage.value = false
    scheduleRender()
  }
  image.src = props.imageUrl
}

function resizeStage() {
  if (!stage || !container.value) return
  stage.size({ width: container.value.clientWidth, height: container.value.clientHeight })
  scheduleRender()
}

function scheduleRender() {
  cancelAnimationFrame(renderFrame)
  renderFrame = requestAnimationFrame(renderScene)
}

function renderScene() {
  if (!stage || !layer) return
  layer.destroyChildren()
  const cropMode = props.tool === 'crop'
  const workspaceWidth = cropMode ? props.sourceWidth : props.document.canvas.width
  const workspaceHeight = cropMode ? props.sourceHeight : props.document.canvas.height
  if (workspaceWidth <= 0 || workspaceHeight <= 0) return
  const margin = stage.width() < 700 ? 24 : 72
  const fitScale = Math.min(
    Math.max(0.01, (stage.width() - margin * 2) / workspaceWidth),
    Math.max(0.01, (stage.height() - margin * 2) / workspaceHeight),
  )
  const scale = Math.max(0.01, Math.min(8, fitScale * zoomMultiplier))
  const workspace = new Konva.Group({
    x: (stage.width() - workspaceWidth * scale) / 2 + panX,
    y: (stage.height() - workspaceHeight * scale) / 2 + panY,
    scaleX: scale,
    scaleY: scale,
  })
  layer.add(workspace)
  emit('zoom', Math.round(scale * 100))
  if (cropMode) renderCropScene(workspace)
  else renderCanvasScene(workspace)
  layer.draw()
}

function renderCropScene(workspace: Konva.Group) {
  workspace.add(new Konva.Rect({
    width: props.sourceWidth,
    height: props.sourceHeight,
    fill: '#11131a',
    shadowColor: '#000',
    shadowBlur: 24,
    shadowOpacity: 0.32,
    listening: false,
  }))
  if (sourceImage) {
    workspace.add(new Konva.Image({
      image: sourceImage,
      width: props.sourceWidth,
      height: props.sourceHeight,
      listening: false,
    }))
  }
  const crop = props.document.image.crop
  const selection = new Konva.Rect({
    x: crop.x,
    y: crop.y,
    width: crop.width,
    height: crop.height,
    fill: 'rgba(255,255,255,0.01)',
    stroke: '#a78bfa',
    strokeWidth: 2 / workspace.scaleX(),
    draggable: true,
  })
  selection.dragBoundFunc((position) => ({
    x: clamp(position.x, 0, props.sourceWidth - selection.width()),
    y: clamp(position.y, 0, props.sourceHeight - selection.height()),
  }))
  selection.on('dragend', () => commitCrop(selection))
  selection.on('transformend', () => commitCrop(selection))
  workspace.add(selection)
  const transformer = new Konva.Transformer({
    nodes: [selection],
    rotateEnabled: false,
    keepRatio: false,
    flipEnabled: false,
    borderStroke: '#a78bfa',
    anchorFill: '#fff',
    anchorStroke: '#7c3aed',
    anchorSize: Math.max(7, 10 / workspace.scaleX()),
    boundBoxFunc: (_oldBox, nextBox) => {
      const width = clamp(nextBox.width, 16, props.sourceWidth)
      const height = clamp(nextBox.height, 16, props.sourceHeight)
      return {
        ...nextBox,
        x: clamp(nextBox.x, 0, props.sourceWidth - width),
        y: clamp(nextBox.y, 0, props.sourceHeight - height),
        width,
        height,
      }
    },
  })
  workspace.add(transformer)
}

function commitCrop(selection: Konva.Rect) {
  const width = clamp(Math.round(selection.width() * Math.abs(selection.scaleX())), 16, props.sourceWidth)
  const height = clamp(Math.round(selection.height() * Math.abs(selection.scaleY())), 16, props.sourceHeight)
  const x = clamp(Math.round(selection.x()), 0, props.sourceWidth - width)
  const y = clamp(Math.round(selection.y()), 0, props.sourceHeight - height)
  const next = cloneEditorDocument(props.document)
  next.image.crop = { x, y, width, height }
  emit('commit', next)
}

function renderCanvasScene(workspace: Konva.Group) {
  const { canvas, image } = props.document
  workspace.add(new Konva.Rect({
    width: canvas.width,
    height: canvas.height,
    fill: canvas.background.type === 'color' ? canvas.background.color : '#ffffff00',
    stroke: '#ffffff55',
    strokeWidth: 1 / workspace.scaleX(),
    shadowColor: '#000',
    shadowBlur: 28,
    shadowOpacity: 0.36,
    listening: false,
  }))
  const clipped = new Konva.Group({
    clipX: 0,
    clipY: 0,
    clipWidth: canvas.width,
    clipHeight: canvas.height,
  })
  workspace.add(clipped)
  if (sourceImage && canvas.background.type === 'blurred-image') {
    const crop = coverSourceCrop(props.sourceWidth, props.sourceHeight, canvas.width, canvas.height)
    const background = new Konva.Image({
      image: sourceImage,
      width: canvas.width,
      height: canvas.height,
      crop,
      listening: false,
    })
    background.cache({ pixelRatio: Math.min(1, 1024 / Math.max(canvas.width, canvas.height)) })
    background.filters([Konva.Filters.Blur])
    background.blurRadius(canvas.background.blurRadius)
    clipped.add(background)
  }
  let imageNode: Konva.Image | null = null
  if (sourceImage) {
    imageNode = new Konva.Image({
      image: sourceImage,
      x: image.x,
      y: image.y,
      width: image.crop.width,
      height: image.crop.height,
      crop: image.crop,
      scaleX: image.flipX ? -image.scaleX : image.scaleX,
      scaleY: image.flipY ? -image.scaleY : image.scaleY,
      offsetX: image.flipX ? image.crop.width : 0,
      offsetY: image.flipY ? image.crop.height : 0,
      rotation: image.rotation,
      draggable: true,
    })
    imageNode.on('dragend transformend', () => commitImageTransform(imageNode!))
    clipped.add(imageNode)
  }
  const safeInsetX = canvas.width * 0.05
  const safeInsetY = canvas.height * 0.05
  workspace.add(new Konva.Rect({
    x: safeInsetX,
    y: safeInsetY,
    width: canvas.width - safeInsetX * 2,
    height: canvas.height - safeInsetY * 2,
    stroke: 'rgba(255,255,255,.45)',
    dash: [8 / workspace.scaleX(), 6 / workspace.scaleX()],
    strokeWidth: 1 / workspace.scaleX(),
    listening: false,
  }))
  if (imageNode && ['select', 'canvas'].includes(props.tool)) {
    workspace.add(new Konva.Transformer({
      nodes: [imageNode],
      keepRatio: true,
      flipEnabled: false,
      borderStroke: '#a78bfa',
      anchorFill: '#fff',
      anchorStroke: '#7c3aed',
      anchorSize: Math.max(7, 10 / workspace.scaleX()),
      rotateAnchorOffset: Math.max(30, 42 / workspace.scaleX()),
    }))
  }
}

function commitImageTransform(node: Konva.Image) {
  const next = cloneEditorDocument(props.document)
  next.layout.fitStrategy = 'free'
  next.image.x = roundTransform(node.x())
  next.image.y = roundTransform(node.y())
  next.image.scaleX = roundTransform(Math.abs(node.scaleX()))
  next.image.scaleY = roundTransform(Math.abs(node.scaleY()))
  next.image.rotation = roundTransform(normalizeRotation(node.rotation()))
  emit('commit', next)
}

function handleWheel(event: Konva.KonvaEventObject<WheelEvent>) {
  event.evt.preventDefault()
  zoomMultiplier = clamp(zoomMultiplier * (event.evt.deltaY > 0 ? 0.9 : 1.1), 0.2, 8)
  scheduleRender()
}

function beginPan(event: Konva.KonvaEventObject<PointerEvent>) {
  if (!(event.evt.altKey || event.evt.button === 1)) return
  event.evt.preventDefault()
  panning = true
  panPointer = { x: event.evt.clientX, y: event.evt.clientY }
}

function movePan(event: Konva.KonvaEventObject<PointerEvent>) {
  if (!panning) return
  panX += event.evt.clientX - panPointer.x
  panY += event.evt.clientY - panPointer.y
  panPointer = { x: event.evt.clientX, y: event.evt.clientY }
  scheduleRender()
}

function endPan() {
  panning = false
}

function fitViewport() {
  zoomMultiplier = 1
  panX = 0
  panY = 0
  scheduleRender()
}

function coverSourceCrop(sourceWidth: number, sourceHeight: number, width: number, height: number) {
  const sourceRatio = sourceWidth / sourceHeight
  const targetRatio = width / height
  if (sourceRatio > targetRatio) {
    const cropWidth = sourceHeight * targetRatio
    return { x: (sourceWidth - cropWidth) / 2, y: 0, width: cropWidth, height: sourceHeight }
  }
  const cropHeight = sourceWidth / targetRatio
  return { x: 0, y: (sourceHeight - cropHeight) / 2, width: sourceWidth, height: cropHeight }
}

function normalizeRotation(rotation: number) {
  return ((rotation % 360) + 360) % 360
}

function roundTransform(value: number) {
  return Math.round(value * 10_000) / 10_000
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

defineExpose({ fitViewport })
</script>

<template>
  <div ref="container" class="editor-canvas" :class="{ loading: loadingImage }">
    <div v-if="loadingImage" class="editor-canvas-loading">正在读取原始清晰图片…</div>
    <div v-else-if="!sourceImage" class="editor-canvas-loading error">原图加载失败</div>
    <div class="editor-canvas-tip">滚轮缩放 · Alt/中键拖动画布</div>
  </div>
</template>

<style scoped>
.editor-canvas {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 420px;
  overflow: hidden;
  background-color: #202127;
  background-image:
    linear-gradient(45deg, #292a31 25%, transparent 25%),
    linear-gradient(-45deg, #292a31 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, #292a31 75%),
    linear-gradient(-45deg, transparent 75%, #292a31 75%);
  background-position: 0 0, 0 12px, 12px -12px, -12px 0;
  background-size: 24px 24px;
}
.editor-canvas-loading {
  position: absolute;
  z-index: 2;
  inset: 0;
  display: grid;
  place-items: center;
  color: #d6d3e4;
  background: rgba(22, 23, 29, .72);
  font-size: 13px;
  backdrop-filter: blur(6px);
}
.editor-canvas-loading.error { color: #fca5a5; }
.editor-canvas-tip {
  position: absolute;
  z-index: 1;
  bottom: 12px;
  left: 50%;
  padding: 6px 10px;
  border: 1px solid rgba(255,255,255,.1);
  border-radius: 999px;
  color: rgba(255,255,255,.62);
  background: rgba(12,13,18,.68);
  font-size: 10px;
  pointer-events: none;
  transform: translateX(-50%);
}
</style>
