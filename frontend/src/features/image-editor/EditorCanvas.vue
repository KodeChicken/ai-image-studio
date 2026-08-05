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
  cropAspectLocked: boolean
}>()

const emit = defineEmits<{
  commit: [document: ImageEditorDocumentV1]
  zoom: [percent: number]
}>()

const container = ref<HTMLDivElement | null>(null)
const stageHost = ref<HTMLDivElement | null>(null)
const loadingImage = ref(true)
const imageError = ref('')
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
let imageLoadTimer: number | null = null
let imageLoadVersion = 0

onMounted(() => {
  if (!container.value || !stageHost.value) return
  stage = new Konva.Stage({ container: stageHost.value, width: 1, height: 1 })
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
  imageLoadVersion += 1
  clearImageLoadTimer()
  cancelAnimationFrame(renderFrame)
  observer?.disconnect()
  sourceImage = null
  stage?.destroy()
})

watch(() => props.imageUrl, loadImage)
watch(
  () => [props.document, props.tool, props.sourceWidth, props.sourceHeight, props.cropAspectLocked],
  scheduleRender,
  { deep: true },
)

function loadImage() {
  const loadVersion = ++imageLoadVersion
  clearImageLoadTimer()
  loadingImage.value = true
  imageError.value = ''
  sourceImage = null
  if (!props.imageUrl) {
    loadingImage.value = false
    imageError.value = '原图地址无效'
    return
  }
  const image = new Image()
  image.onload = () => {
    if (loadVersion !== imageLoadVersion) return
    clearImageLoadTimer()
    sourceImage = image
    loadingImage.value = false
    scheduleRender()
  }
  image.onerror = () => {
    if (loadVersion !== imageLoadVersion) return
    clearImageLoadTimer()
    loadingImage.value = false
    imageError.value = '原图加载失败，请检查网络后重试'
    scheduleRender()
  }
  image.src = props.imageUrl
  imageLoadTimer = window.setTimeout(() => {
    if (loadVersion !== imageLoadVersion) return
    image.onload = null
    image.onerror = null
    image.src = ''
    loadingImage.value = false
    imageError.value = '原图加载超时，请重试'
    scheduleRender()
  }, 60_000)
}

function clearImageLoadTimer() {
  if (imageLoadTimer !== null) window.clearTimeout(imageLoadTimer)
  imageLoadTimer = null
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
  selection.dragBoundFunc((position) => {
    const local = absoluteToLocal(workspace, position)
    return localToAbsolute(workspace, {
      x: clamp(local.x, 0, props.sourceWidth - selection.width()),
      y: clamp(local.y, 0, props.sourceHeight - selection.height()),
    })
  })
  selection.on('dragend', () => commitCrop(selection))
  selection.on('transformend', () => commitCrop(selection))
  workspace.add(selection)
  const transformer = new Konva.Transformer({
    nodes: [selection],
    rotateEnabled: false,
    keepRatio: props.cropAspectLocked,
    shiftBehavior: props.cropAspectLocked ? 'inverted' : 'default',
    flipEnabled: false,
    borderStroke: '#a78bfa',
    anchorFill: '#fff',
    anchorStroke: '#7c3aed',
    anchorSize: Math.max(7, 10 / workspace.scaleX()),
    boundBoxFunc: (oldBox, nextBox) => {
      const localTopLeft = absoluteToLocal(workspace, { x: nextBox.x, y: nextBox.y })
      const localBottomRight = absoluteToLocal(workspace, {
        x: nextBox.x + nextBox.width,
        y: nextBox.y + nextBox.height,
      })
      let width = clamp(localBottomRight.x - localTopLeft.x, 16, props.sourceWidth)
      let height = clamp(localBottomRight.y - localTopLeft.y, 16, props.sourceHeight)
      if (props.cropAspectLocked) {
        const ratio = props.document.image.crop.width / props.document.image.crop.height
        if (width / height > ratio) width = height * ratio
        else height = width / ratio
        const sourceScale = Math.min(1, props.sourceWidth / width, props.sourceHeight / height)
        width *= sourceScale
        height *= sourceScale
        if (width < 16 || height < 16) return oldBox
      }
      const topLeft = localToAbsolute(workspace, {
        x: clamp(localTopLeft.x, 0, props.sourceWidth - width),
        y: clamp(localTopLeft.y, 0, props.sourceHeight - height),
      })
      return {
        ...nextBox,
        x: topLeft.x,
        y: topLeft.y,
        width: width * workspace.getAbsoluteScale().x,
        height: height * workspace.getAbsoluteScale().y,
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
      draggable: ['select', 'canvas'].includes(props.tool),
    })
    imageNode.dragBoundFunc((position) => {
      const local = absoluteToLocal(workspace, position)
      return localToAbsolute(
        workspace,
        snapImagePosition(local, image, canvas, workspace.getAbsoluteScale().x),
      )
    })
    imageNode.on('dragend transformend', () => commitImageTransform(imageNode!))
    clipped.add(imageNode)
  }
  const safeInsetX = canvas.width * 0.05
  const safeInsetY = canvas.height * 0.05
  workspace.add(new Konva.Line({
    points: [canvas.width / 2, 0, canvas.width / 2, canvas.height],
    stroke: 'rgba(167,139,250,.35)',
    dash: [5 / workspace.scaleX(), 5 / workspace.scaleX()],
    strokeWidth: 1 / workspace.scaleX(),
    listening: false,
  }))
  workspace.add(new Konva.Line({
    points: [0, canvas.height / 2, canvas.width, canvas.height / 2],
    stroke: 'rgba(167,139,250,.35)',
    dash: [5 / workspace.scaleX(), 5 / workspace.scaleX()],
    strokeWidth: 1 / workspace.scaleX(),
    listening: false,
  }))
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
      shiftBehavior: 'inverted',
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

function snapImagePosition(
  position: { x: number; y: number },
  image: ImageEditorDocumentV1['image'],
  canvas: ImageEditorDocumentV1['canvas'],
  viewportScale: number,
) {
  if (normalizeRotation(image.rotation) !== 0) return position
  const threshold = 8 / viewportScale
  const width = image.crop.width * image.scaleX
  const height = image.crop.height * image.scaleY
  return {
    x: snapTo(position.x, [0, (canvas.width - width) / 2, canvas.width - width], threshold),
    y: snapTo(position.y, [0, (canvas.height - height) / 2, canvas.height - height], threshold),
  }
}

function snapTo(value: number, targets: number[], threshold: number) {
  return targets.find((target) => Math.abs(target - value) <= threshold) ?? value
}

function absoluteToLocal(workspace: Konva.Group, point: { x: number; y: number }) {
  return workspace.getAbsoluteTransform().copy().invert().point(point)
}

function localToAbsolute(workspace: Konva.Group, point: { x: number; y: number }) {
  return workspace.getAbsoluteTransform().point(point)
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
    <div ref="stageHost" class="editor-canvas-stage" aria-hidden="true"></div>
    <div v-if="loadingImage" class="editor-canvas-loading">正在读取原始清晰图片…</div>
    <div v-else-if="!sourceImage" class="editor-canvas-loading error" role="alert">
      <span>{{ imageError || '原图加载失败' }}</span>
      <button type="button" @click="loadImage">重新加载</button>
    </div>
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
.editor-canvas-stage { position: absolute; inset: 0; }
.editor-canvas-loading.error { align-content: center; justify-items: center; gap: 12px; color: #fca5a5; }
.editor-canvas-loading.error button { padding: 7px 12px; border: 1px solid #ffffff24; border-radius: 8px; cursor: pointer; color: #fff; background: #7c3aed; }
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
