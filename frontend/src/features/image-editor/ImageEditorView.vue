<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, useRoute, useRouter } from 'vue-router'
import { NButton, NInput, NSelect, NSwitch, useMessage } from 'naive-ui'
import { api, streamTask } from '@/api/client'
import type { ImageEditCapability, ImageModel } from '@/types/api'
import EditorCanvas, { type EditorTool } from './EditorCanvas.vue'
import {
  assertCanvasSize,
  assertCropRect,
  cloneEditorDocument,
  createEditorDocument,
  cropAsCanvas,
  parseEditorDocument,
  type EditorAnchor,
  type EditorAsset,
  type EditorBackground,
  type EditorFitStrategy,
  type ImageEditDocumentResponse,
  type ImageEditorDocumentV1,
} from './editorDocument'
import { EditorHistory } from './editorHistory'
import { applyFitStrategy } from './fitStrategies'
import { renderEditorDocument, type EditorExportFormat } from './exporter'

const route = useRoute()
const router = useRouter()
const message = useMessage()
const loading = ref(true)
const loadError = ref('')
const documentView = ref<ImageEditDocumentResponse | null>(null)
const editorDocument = ref<ImageEditorDocumentV1 | null>(null)
const currentAsset = ref<EditorAsset | null>(null)
const initialDocument = ref<ImageEditorDocumentV1 | null>(null)
const version = ref(0)
const tool = ref<EditorTool>('select')
const zoom = ref(100)
const saving = ref(false)
const saveState = ref<'saved' | 'dirty' | 'saving' | 'error'>('saved')
const savedFingerprint = ref('')
const history = new EditorHistory(100)
const historyRevision = ref(0)
const canvasRef = ref<InstanceType<typeof EditorCanvas> | null>(null)
let saveTimer: ReturnType<typeof setTimeout> | null = null
let dimensionFrame = 0
let inputSessionStart: ImageEditorDocumentV1 | null = null

const widthDraft = ref('')
const heightDraft = ref('')
const cropXDraft = ref('')
const cropYDraft = ref('')
const cropWidthDraft = ref('')
const cropHeightDraft = ref('')
const activeInput = ref<string | null>(null)
const dimensionError = ref('')
const cropError = ref('')
const aspectLocked = ref(false)
const lockedRatio = ref(1)
const exportFormat = ref<EditorExportFormat>('png')
const exporting = ref(false)
const exportResult = ref<EditorAsset | null>(null)
const models = ref<ImageModel[]>([])
const aiModelId = ref<string | null>(null)
const aiPrompt = ref('')
const aiRunning = ref(false)
const aiStage = ref('')
const assetCache = new Map<string, EditorAsset>()

const canUndo = computed(() => (historyRevision.value >= 0 && history.canUndo))
const canRedo = computed(() => (historyRevision.value >= 0 && history.canRedo))
const sourceAsset = computed(() => documentView.value?.sourceAsset ?? null)
const imageUrl = computed(() => currentAsset.value?.contentUrl ?? '')
const selectedAiModel = computed(() => models.value.find((model) => model.id === aiModelId.value) ?? null)
const outpaintModels = computed(() => models.value.filter((model) => {
  const capability = imageEditCapability(model)
  return model.enabled
    && capability?.supportsImageEdit
    && capability.supportsOutpaint
    && Boolean(currentAsset.value && capability.supportedInputMimeTypes.includes(currentAsset.value.mimeType))
}))
const aiModelOptions = computed(() => outpaintModels.value.map((model) => ({
  label: `${model.displayName} · ${model.providerType}`,
  value: model.id,
})))
const statusLabel = computed(() => ({
  saved: '已保存', dirty: '有未保存修改', saving: '保存中…', error: '保存失败',
})[saveState.value])

onMounted(async () => {
  window.addEventListener('beforeunload', protectUnsavedChanges)
  window.addEventListener('keydown', handleShortcut)
  await loadEditor()
})

onBeforeUnmount(() => {
  window.removeEventListener('beforeunload', protectUnsavedChanges)
  window.removeEventListener('keydown', handleShortcut)
  if (saveTimer) clearTimeout(saveTimer)
  cancelAnimationFrame(dimensionFrame)
})

onBeforeRouteLeave(() => {
  if (saveState.value === 'dirty' || saveState.value === 'error') {
    return window.confirm('编辑内容尚未保存，确定离开吗？')
  }
  return true
})

watch(editorDocument, syncDrafts, { deep: true })
watch(() => editorDocument.value?.image.assetId, (assetId) => {
  if (assetId && assetCache.has(assetId)) currentAsset.value = assetCache.get(assetId)!
})
watch(aspectLocked, (locked) => {
  if (locked && editorDocument.value) {
    lockedRatio.value = editorDocument.value.canvas.width / editorDocument.value.canvas.height
  }
})

async function loadEditor() {
  loading.value = true
  loadError.value = ''
  try {
    const assetId = String(route.params.assetId ?? '')
    const documentId = typeof route.query.documentId === 'string' ? route.query.documentId : null
    const mode = typeof route.query.mode === 'string' ? route.query.mode : 'canvas'
    const loaded = documentId
      ? await api<ImageEditDocumentResponse>(`/api/v1/image-edit-documents/${documentId}`)
      : await api<ImageEditDocumentResponse>('/api/v1/image-edit-documents', {
          method: 'POST',
          body: JSON.stringify({ sourceAssetId: assetId, mode }),
        })
    documentView.value = loaded
    assetCache.set(loaded.sourceAsset.id, loaded.sourceAsset)
    assetCache.set(loaded.imageAsset.id, loaded.imageAsset)
    currentAsset.value = loaded.imageAsset
    editorDocument.value = parseEditorDocument(loaded.document, loaded.imageAsset)
    initialDocument.value = createEditorDocument(
      loaded.sourceAsset.id,
      loaded.sourceAsset.width,
      loaded.sourceAsset.height,
    )
    version.value = loaded.version
    savedFingerprint.value = fingerprint(editorDocument.value)
    saveState.value = 'saved'
    tool.value = mode === 'crop' ? 'crop' : mode === 'expand' ? 'ai' : 'select'
    if (!documentId) {
      await router.replace({
        path: `/editor/${loaded.sourceAssetId}`,
        query: { documentId: loaded.id, ...(mode !== 'canvas' ? { mode } : {}) },
      })
    }
    models.value = await api<ImageModel[]>('/api/v1/models?includeDiscovered=true&imageOnly=true')
    aiModelId.value = outpaintModels.value[0]?.id ?? null
    syncDrafts()
  } catch (error) {
    loadError.value = error instanceof Error ? error.message : '图片编辑器加载失败'
  } finally {
    loading.value = false
  }
}

function commitDocument(next: ImageEditorDocumentV1, recordHistory = true) {
  if (!editorDocument.value || fingerprint(next) === fingerprint(editorDocument.value)) return
  if (recordHistory) history.push(editorDocument.value)
  editorDocument.value = cloneEditorDocument(next)
  historyRevision.value += 1
  markDirty()
}

function commitCanvasChange(next: ImageEditorDocumentV1) {
  commitDocument(tool.value === 'crop' ? cropAsCanvas(next) : next)
}

function markDirty() {
  saveState.value = 'dirty'
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => void saveDocument(false).catch(() => undefined), 900)
}

async function saveDocument(showSuccess = true): Promise<void> {
  if (!editorDocument.value || !documentView.value) return
  if (saving.value) {
    markDirty()
    return
  }
  const snapshot = cloneEditorDocument(editorDocument.value)
  const snapshotFingerprint = fingerprint(snapshot)
  if (snapshotFingerprint === savedFingerprint.value) {
    saveState.value = 'saved'
    return
  }
  saving.value = true
  saveState.value = 'saving'
  try {
    const saved = await api<ImageEditDocumentResponse>(
      `/api/v1/image-edit-documents/${documentView.value.id}`,
      {
        method: 'PUT',
        body: JSON.stringify({ version: version.value, schemaVersion: 1, document: snapshot }),
      },
    )
    version.value = saved.version
    documentView.value = saved
    savedFingerprint.value = snapshotFingerprint
    if (editorDocument.value && fingerprint(editorDocument.value) === snapshotFingerprint) {
      saveState.value = 'saved'
      if (showSuccess) message.success('编辑文档已保存')
    } else {
      markDirty()
    }
  } catch (error) {
    saveState.value = 'error'
    if (showSuccess) message.error(error instanceof Error ? error.message : '保存失败')
    throw error
  } finally {
    saving.value = false
  }
}

function undo() {
  if (!editorDocument.value) return
  const previous = history.undo(editorDocument.value)
  if (!previous) return
  editorDocument.value = previous
  historyRevision.value += 1
  markDirty()
}

function redo() {
  if (!editorDocument.value) return
  const next = history.redo(editorDocument.value)
  if (!next) return
  editorDocument.value = next
  historyRevision.value += 1
  markDirty()
}

function resetDocument() {
  if (initialDocument.value) commitDocument(initialDocument.value)
}

function beginInput(name: string) {
  activeInput.value = name
  inputSessionStart = editorDocument.value ? cloneEditorDocument(editorDocument.value) : null
}

function finishInput(name: string) {
  if (activeInput.value === name) activeInput.value = null
  if (inputSessionStart && editorDocument.value
    && fingerprint(inputSessionStart) !== fingerprint(editorDocument.value)) {
    history.push(inputSessionStart)
    historyRevision.value += 1
  }
  inputSessionStart = null
}

function updateCanvasDraft(edge: 'width' | 'height', value: string) {
  if (edge === 'width') widthDraft.value = value
  else heightDraft.value = value
  if (!editorDocument.value) return
  const entered = parseExactInteger(value)
  if (entered === null) {
    dimensionError.value = '请输入 16–8192 的整数像素值'
    return
  }
  let width = edge === 'width' ? entered : editorDocument.value.canvas.width
  let height = edge === 'height' ? entered : editorDocument.value.canvas.height
  if (aspectLocked.value) {
    if (edge === 'width') height = Math.round(width / lockedRatio.value)
    else width = Math.round(height * lockedRatio.value)
  }
  try {
    assertCanvasSize(width, height)
    dimensionError.value = ''
  } catch (error) {
    dimensionError.value = error instanceof Error ? error.message : '尺寸无效'
    return
  }
  cancelAnimationFrame(dimensionFrame)
  dimensionFrame = requestAnimationFrame(() => {
    if (!editorDocument.value) return
    const next = cloneEditorDocument(editorDocument.value)
    next.canvas.width = width
    next.canvas.height = height
    const fitted = next.layout.fitStrategy === 'free' ? next : applyFitStrategy(next)
    commitDocument(fitted, false)
    if (aspectLocked.value) {
      if (edge === 'width' && activeInput.value !== 'height') heightDraft.value = String(height)
      if (edge === 'height' && activeInput.value !== 'width') widthDraft.value = String(width)
    }
  })
}

function updateCropDraft(field: 'x' | 'y' | 'width' | 'height', value: string) {
  const drafts = { x: cropXDraft, y: cropYDraft, width: cropWidthDraft, height: cropHeightDraft }
  drafts[field].value = value
  if (!editorDocument.value || !currentAsset.value) return
  const crop = {
    x: parseExactInteger(cropXDraft.value, true),
    y: parseExactInteger(cropYDraft.value, true),
    width: parseExactInteger(cropWidthDraft.value),
    height: parseExactInteger(cropHeightDraft.value),
  }
  if (Object.values(crop).some((number) => number === null)) {
    cropError.value = '裁剪位置和尺寸必须是整数'
    return
  }
  try {
    assertCropRect(crop as { x: number; y: number; width: number; height: number }, currentAsset.value.width, currentAsset.value.height)
    cropError.value = ''
  } catch (error) {
    cropError.value = error instanceof Error ? error.message : '裁剪区域无效'
    return
  }
  cancelAnimationFrame(dimensionFrame)
  dimensionFrame = requestAnimationFrame(() => {
    if (!editorDocument.value) return
    const next = cloneEditorDocument(editorDocument.value)
    next.image.crop = crop as { x: number; y: number; width: number; height: number }
    commitDocument(tool.value === 'crop' ? cropAsCanvas(next) : next, false)
  })
}

function applyCanvasPreset(width: number, height: number) {
  if (!editorDocument.value) return
  const next = cloneEditorDocument(editorDocument.value)
  next.canvas.width = width
  next.canvas.height = height
  commitDocument(next.layout.fitStrategy === 'free' ? next : applyFitStrategy(next))
}

function swapCanvasEdges() {
  if (editorDocument.value) applyCanvasPreset(editorDocument.value.canvas.height, editorDocument.value.canvas.width)
}

function setFitStrategy(value: EditorFitStrategy) {
  if (editorDocument.value) commitDocument(applyFitStrategy(editorDocument.value, value))
}

function setAnchor(value: EditorAnchor) {
  if (editorDocument.value) commitDocument(applyFitStrategy(editorDocument.value, editorDocument.value.layout.fitStrategy, value))
}

function setBackground(type: EditorBackground['type']) {
  if (!editorDocument.value) return
  const next = cloneEditorDocument(editorDocument.value)
  next.canvas.background = type === 'color'
    ? { type, color: '#ffffff' }
    : type === 'blurred-image'
      ? { type, blurRadius: 24 }
      : { type }
  commitDocument(next)
}

function setBackgroundColor(color: string) {
  if (!editorDocument.value || editorDocument.value.canvas.background.type !== 'color') return
  const next = cloneEditorDocument(editorDocument.value)
  next.canvas.background = { type: 'color', color }
  commitDocument(next)
}

function setBlurRadius(value: string) {
  const blurRadius = Number(value)
  if (!editorDocument.value || !Number.isFinite(blurRadius)) return
  const next = cloneEditorDocument(editorDocument.value)
  next.canvas.background = { type: 'blurred-image', blurRadius: Math.max(0, Math.min(100, blurRadius)) }
  commitDocument(next)
}

function rotateBy(degrees: number) {
  if (!editorDocument.value) return
  const next = cloneEditorDocument(editorDocument.value)
  next.layout.fitStrategy = 'free'
  next.image.rotation = ((next.image.rotation + degrees) % 360 + 360) % 360
  commitDocument(next)
}

function toggleFlip(edge: 'x' | 'y') {
  if (!editorDocument.value) return
  const next = cloneEditorDocument(editorDocument.value)
  next.layout.fitStrategy = 'free'
  if (edge === 'x') next.image.flipX = !next.image.flipX
  else next.image.flipY = !next.image.flipY
  commitDocument(next)
}

async function exportImage() {
  if (!editorDocument.value || !documentView.value || !currentAsset.value) return
  exporting.value = true
  exportResult.value = null
  try {
    await saveDocument(false)
    const blob = await renderEditorDocument(editorDocument.value, currentAsset.value.contentUrl, {
      format: exportFormat.value,
      quality: 0.95,
    })
    const form = new FormData()
    const extension = exportFormat.value === 'jpeg' ? 'jpg' : exportFormat.value
    form.append('file', blob, `ai-image-studio-${documentView.value.id}.${extension}`)
    form.append('documentVersion', String(version.value))
    form.append('format', exportFormat.value)
    form.append('width', String(editorDocument.value.canvas.width))
    form.append('height', String(editorDocument.value.canvas.height))
    exportResult.value = await api<EditorAsset>(
      `/api/v1/image-edit-documents/${documentView.value.id}/exports`,
      { method: 'POST', body: form },
    )
    message.success(`已导出 ${editorDocument.value.canvas.width} × ${editorDocument.value.canvas.height} 清晰成品`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '导出失败，编辑状态已保留')
  } finally {
    exporting.value = false
  }
}

async function runAiExpand() {
  if (!documentView.value || !selectedAiModel.value || !editorDocument.value) return
  aiRunning.value = true
  aiStage.value = '正在创建扩图任务…'
  try {
    await saveDocument(false)
    const created = await api<{ taskId: string }>(
      `/api/v1/image-edit-documents/${documentView.value.id}/ai-expand`,
      {
        method: 'POST',
        body: JSON.stringify({
          providerId: selectedAiModel.value.providerId,
          modelId: selectedAiModel.value.id,
          prompt: aiPrompt.value,
          parameters: {},
        }),
      },
    )
    await streamTask(created.taskId, (event) => {
      if (event.type === 'task.progress') aiStage.value = stageLabel(String(event.data.stage ?? 'processing'))
    })
    const task = await api<{ results: EditorAsset[]; errorMessage?: string | null }>(`/api/v1/tasks/${created.taskId}`)
    const result = task.results[0]
    if (!result || !editorDocument.value) throw new Error(task.errorMessage ?? 'AI 扩图没有返回图片')
    assetCache.set(result.id, result)
    currentAsset.value = result
    const next = cloneEditorDocument(editorDocument.value)
    next.image.assetId = result.id
    next.image.crop = { x: 0, y: 0, width: result.width, height: result.height }
    commitDocument(applyFitStrategy(next, 'cover'))
    message.success('AI 扩图完成，结果已作为新的派生图片加入当前文档')
  } catch (error) {
    message.error(error instanceof Error ? error.message : 'AI 扩图失败，当前画布未被替换')
  } finally {
    aiRunning.value = false
    aiStage.value = ''
  }
}

function imageEditCapability(model: ImageModel): ImageEditCapability | null {
  const value = model.capabilities.image_edit_capability
  if (!value || typeof value !== 'object') return null
  return value as ImageEditCapability
}

function syncDrafts() {
  const document = editorDocument.value
  if (!document) return
  if (activeInput.value !== 'width') widthDraft.value = String(document.canvas.width)
  if (activeInput.value !== 'height') heightDraft.value = String(document.canvas.height)
  if (activeInput.value !== 'crop-x') cropXDraft.value = String(Math.round(document.image.crop.x))
  if (activeInput.value !== 'crop-y') cropYDraft.value = String(Math.round(document.image.crop.y))
  if (activeInput.value !== 'crop-width') cropWidthDraft.value = String(Math.round(document.image.crop.width))
  if (activeInput.value !== 'crop-height') cropHeightDraft.value = String(Math.round(document.image.crop.height))
}

function parseExactInteger(value: string, allowZero = false): number | null {
  if (!/^\d+$/.test(value)) return null
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < (allowZero ? 0 : 16) || parsed > 8192) return null
  return parsed
}

function handleShortcut(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null
  const typing = target?.matches('input, textarea, [contenteditable="true"]')
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') {
    event.preventDefault()
    if (event.shiftKey) redo()
    else undo()
    return
  }
  if (typing || !editorDocument.value) return
  const movement: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1],
  }
  const delta = movement[event.key]
  if (!delta) return
  event.preventDefault()
  const next = cloneEditorDocument(editorDocument.value)
  const multiplier = event.shiftKey ? 10 : 1
  next.layout.fitStrategy = 'free'
  next.image.x += delta[0] * multiplier
  next.image.y += delta[1] * multiplier
  commitDocument(next)
}

function protectUnsavedChanges(event: BeforeUnloadEvent) {
  if (saveState.value === 'dirty' || saveState.value === 'error') event.preventDefault()
}

function stageLabel(stage: string) {
  if (stage.includes('provider')) return '模型正在延展画面…'
  if (stage.includes('storage')) return '正在校验并保存扩图结果…'
  if (stage.includes('retry')) return '模型请求重试中…'
  return 'AI 扩图处理中…'
}

function fingerprint(document: ImageEditorDocumentV1) {
  return JSON.stringify(document)
}
</script>

<template>
  <main class="image-editor-page">
    <div v-if="loading" class="editor-loading">正在打开原始清晰图片…</div>
    <div v-else-if="loadError" class="editor-loading error">
      <strong>无法打开图片编辑器</strong><span>{{ loadError }}</span><n-button @click="router.back()">返回</n-button>
    </div>
    <template v-else-if="editorDocument && documentView && currentAsset">
      <header class="editor-topbar">
        <div class="editor-file">
          <button type="button" aria-label="返回" @click="router.back()">←</button>
          <div><strong>{{ documentView.title }}</strong><span :class="saveState">{{ statusLabel }}</span></div>
        </div>
        <div class="editor-history-actions">
          <n-button quaternary :disabled="!canUndo" @click="undo">↶ 撤销</n-button>
          <n-button quaternary :disabled="!canRedo" @click="redo">↷ 重做</n-button>
          <n-button quaternary @click="resetDocument">恢复原图</n-button>
        </div>
        <div class="editor-export-actions">
          <span>{{ zoom }}%</span>
          <n-button quaternary @click="canvasRef?.fitViewport()">适应窗口</n-button>
          <n-select
            v-model:value="exportFormat" class="export-format" :options="[
              { label: 'PNG', value: 'png' }, { label: 'JPEG', value: 'jpeg' }, { label: 'WebP', value: 'webp' },
            ]"
          />
          <n-button :loading="saving" @click="saveDocument()">保存</n-button>
          <n-button type="primary" :loading="exporting" @click="exportImage">导出成品</n-button>
        </div>
      </header>

      <section class="editor-body">
        <nav class="editor-toolbar" aria-label="图片编辑工具">
          <button
            v-for="item in [
              { id: 'select', icon: '⌖', label: '选择' },
              { id: 'crop', icon: '⌗', label: '裁剪' },
              { id: 'canvas', icon: '▣', label: '画布' },
              { id: 'background', icon: '◐', label: '背景' },
              { id: 'ai', icon: '✦', label: 'AI 扩图' },
            ]" :key="item.id" type="button" :class="{ active: tool === item.id }" @click="tool = item.id as EditorTool"
          >
            <i>{{ item.icon }}</i><span>{{ item.label }}</span>
          </button>
        </nav>

        <div class="editor-stage-wrap">
          <EditorCanvas
            ref="canvasRef"
            :document="editorDocument"
            :image-url="imageUrl"
            :source-width="currentAsset.width"
            :source-height="currentAsset.height"
            :tool="tool"
            @commit="commitCanvasChange"
            @zoom="zoom = $event"
          />
        </div>

        <aside class="editor-inspector">
          <section v-if="tool === 'crop'">
            <header><div><h2>原图裁剪</h2><p>所有数值均为原图真实像素</p></div></header>
            <div class="field-grid four-fields">
              <label>X<input :value="cropXDraft" inputmode="numeric" @focus="beginInput('crop-x')" @blur="finishInput('crop-x')" @input="updateCropDraft('x', ($event.target as HTMLInputElement).value)" /></label>
              <label>Y<input :value="cropYDraft" inputmode="numeric" @focus="beginInput('crop-y')" @blur="finishInput('crop-y')" @input="updateCropDraft('y', ($event.target as HTMLInputElement).value)" /></label>
              <label>宽<input :value="cropWidthDraft" inputmode="numeric" @focus="beginInput('crop-width')" @blur="finishInput('crop-width')" @input="updateCropDraft('width', ($event.target as HTMLInputElement).value)" /></label>
              <label>高<input :value="cropHeightDraft" inputmode="numeric" @focus="beginInput('crop-height')" @blur="finishInput('crop-height')" @input="updateCropDraft('height', ($event.target as HTMLInputElement).value)" /></label>
            </div>
            <p v-if="cropError" class="field-error">{{ cropError }}</p>
            <div class="preset-grid"><button
              v-for="ratio in [
                { label: '1:1', w: 1, h: 1 }, { label: '4:3', w: 4, h: 3 }, { label: '3:4', w: 3, h: 4 },
                { label: '16:9', w: 16, h: 9 }, { label: '9:16', w: 9, h: 16 },
              ]" :key="ratio.label" type="button" @click="updateCropDraft('height', String(Math.min(currentAsset.height, Math.round(editorDocument.image.crop.width * ratio.h / ratio.w))))"
            >{{ ratio.label }}</button></div>
          </section>

          <template v-else-if="tool === 'canvas' || tool === 'select'">
            <section>
              <header><div><h2>成品画布</h2><p>宽高默认独立，不会自动修改</p></div><button type="button" class="swap-button" @click="swapCanvasEdges">⇄</button></header>
              <div class="field-grid dimension-fields">
                <label>宽度<input :value="widthDraft" inputmode="numeric" @focus="beginInput('width')" @blur="finishInput('width')" @keydown.enter="($event.target as HTMLInputElement).blur()" @input="updateCanvasDraft('width', ($event.target as HTMLInputElement).value)" /></label>
                <span>×</span>
                <label>高度<input :value="heightDraft" inputmode="numeric" @focus="beginInput('height')" @blur="finishInput('height')" @keydown.enter="($event.target as HTMLInputElement).blur()" @input="updateCanvasDraft('height', ($event.target as HTMLInputElement).value)" /></label>
              </div>
              <label class="switch-row"><span>锁定宽高比</span><n-switch v-model:value="aspectLocked" /></label>
              <p v-if="dimensionError" class="field-error">{{ dimensionError }}</p>
              <div class="preset-grid"><button
                v-for="preset in [
                  { label: '1:1', w: 1024, h: 1024 }, { label: '16:9', w: 1920, h: 1080 },
                  { label: '9:16', w: 1080, h: 1920 }, { label: '4:3', w: 1600, h: 1200 },
                  { label: '3:4', w: 1080, h: 1440 },
                ]" :key="preset.label" type="button" @click="applyCanvasPreset(preset.w, preset.h)"
              >{{ preset.label }}</button></div>
              <details class="platform-presets"><summary>常用场景尺寸</summary><div>
                <button type="button" @click="applyCanvasPreset(900, 383)">公众号头图 900×383</button>
                <button type="button" @click="applyCanvasPreset(1242, 1660)">小红书 1242×1660</button>
                <button type="button" @click="applyCanvasPreset(1080, 1920)">抖音封面 1080×1920</button>
                <button type="button" @click="applyCanvasPreset(800, 800)">电商主图 800×800</button>
              </div></details>
            </section>
            <section>
              <header><div><h2>图片适配</h2><p>拉伸填满可能使图片变形</p></div></header>
              <n-select
                :value="editorDocument.layout.fitStrategy" :options="[
                  { label: '填满画布（保持比例）', value: 'cover' },
                  { label: '完整显示（保持比例）', value: 'contain' },
                  { label: '自由布局', value: 'free' },
                  { label: '拉伸填满（可能变形）', value: 'stretch' },
                ]" @update:value="setFitStrategy($event as EditorFitStrategy)"
              />
              <label class="select-field">主体对齐<n-select
                :value="editorDocument.layout.anchor" :options="[
                  { label: '左上', value: 'top-left' }, { label: '上', value: 'top' }, { label: '右上', value: 'top-right' },
                  { label: '左', value: 'left' }, { label: '居中', value: 'center' }, { label: '右', value: 'right' },
                  { label: '左下', value: 'bottom-left' }, { label: '下', value: 'bottom' }, { label: '右下', value: 'bottom-right' },
                ]" @update:value="setAnchor($event as EditorAnchor)"
              /></label>
              <div class="transform-actions"><n-button @click="rotateBy(-90)">左转 90°</n-button><n-button @click="rotateBy(90)">右转 90°</n-button><n-button @click="toggleFlip('x')">水平翻转</n-button><n-button @click="toggleFlip('y')">垂直翻转</n-button></div>
              <p class="quality-note">放大不会创造原图细节；导出始终重新读取当前原始 Asset，不使用屏幕预览图。</p>
            </section>
          </template>

          <section v-else-if="tool === 'background'">
            <header><div><h2>画布背景</h2><p>完整显示和自由布局时用于填补空白</p></div></header>
            <div class="background-options">
              <button
                v-for="option in [
                  { label: '透明', value: 'transparent' }, { label: '纯色', value: 'color' }, { label: '模糊原图', value: 'blurred-image' },
                ]" :key="option.value" type="button" :class="{ active: editorDocument.canvas.background.type === option.value }" @click="setBackground(option.value as EditorBackground['type'])"
              >{{ option.label }}</button>
            </div>
            <label v-if="editorDocument.canvas.background.type === 'color'" class="color-field">背景颜色<input type="color" :value="editorDocument.canvas.background.color" @input="setBackgroundColor(($event.target as HTMLInputElement).value)" /></label>
            <label v-if="editorDocument.canvas.background.type === 'blurred-image'" class="range-field">模糊强度<input type="range" min="0" max="100" :value="editorDocument.canvas.background.blurRadius" @input="setBlurRadius(($event.target as HTMLInputElement).value)" /><span>{{ editorDocument.canvas.background.blurRadius }}px</span></label>
          </section>

          <section v-else-if="tool === 'ai'">
            <header><div><h2>AI 扩图</h2><p>只展示明确声明支持扩图的模型</p></div></header>
            <div v-if="outpaintModels.length" class="ai-controls">
              <label>扩图模型<n-select v-model:value="aiModelId" :options="aiModelOptions" /></label>
              <label>补充说明（可留空）<n-input v-model:value="aiPrompt" type="textarea" :rows="4" placeholder="例如：向两侧自然延展海滩和天空，保持人物不变" /></label>
              <n-button type="primary" :loading="aiRunning" block @click="runAiExpand">{{ aiRunning ? aiStage : '开始 AI 扩图' }}</n-button>
              <p>AI 失败不会替换当前图片；成功结果会保存为新的派生 Asset，仍可撤销。</p>
            </div>
            <div v-else class="capability-empty">当前没有模型明确声明支持图片扩图。请先在 Provider 模型能力中完成验证。</div>
          </section>

          <section v-if="exportResult" class="export-result">
            <strong>成品已保存</strong><span>{{ exportResult.width }} × {{ exportResult.height }} · {{ exportResult.mimeType }}</span>
            <a :href="exportResult.contentUrl" download>下载成品</a>
          </section>
        </aside>
      </section>

      <footer class="editor-statusbar">
        <span>初始原图 {{ sourceAsset?.width }} × {{ sourceAsset?.height }}</span>
        <span>当前素材 {{ currentAsset.width }} × {{ currentAsset.height }}</span>
        <span>成品 {{ editorDocument.canvas.width }} × {{ editorDocument.canvas.height }}</span>
        <span>预览 {{ zoom }}%</span>
        <span>sRGB · 原图只读</span>
      </footer>
    </template>
  </main>
</template>

<style scoped>
.image-editor-page { display: grid; height: 100dvh; min-width: 0; grid-template-rows: 58px minmax(0, 1fr) 30px; color: #eeeaf7; background: #17181d; }
.editor-loading { display: grid; height: 100dvh; place-items: center; align-content: center; gap: 14px; color: #c9c4d4; background: #17181d; }
.editor-loading.error strong { color: #fca5a5; font-size: 18px; }
.editor-topbar { display: grid; z-index: 4; grid-template-columns: minmax(180px, 1fr) auto minmax(350px, 1fr); align-items: center; gap: 14px; padding: 0 14px; border-bottom: 1px solid rgba(255,255,255,.08); background: #222329; }
.editor-file, .editor-file > div, .editor-history-actions, .editor-export-actions { display: flex; align-items: center; gap: 8px; }
.editor-file > button { display: grid; width: 34px; height: 34px; place-items: center; padding: 0; border: 1px solid rgba(255,255,255,.1); border-radius: 9px; cursor: pointer; color: inherit; background: transparent; }
.editor-file > div { min-width: 0; align-items: flex-start; flex-direction: column; gap: 1px; }
.editor-file strong { overflow: hidden; max-width: 230px; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; }
.editor-file span, .editor-export-actions > span { color: #96919f; font-size: 10px; }.editor-file span.dirty, .editor-file span.error { color: #fbbf24; }.editor-file span.saved { color: #86efac; }
.editor-history-actions { justify-content: center; }.editor-export-actions { justify-content: flex-end; }.export-format { width: 92px; }
.editor-body { display: grid; min-height: 0; grid-template-columns: 74px minmax(0, 1fr) 320px; }
.editor-toolbar { display: flex; z-index: 3; align-items: stretch; flex-direction: column; gap: 6px; padding: 10px 7px; border-right: 1px solid rgba(255,255,255,.08); background: #202126; }
.editor-toolbar button { display: grid; min-height: 58px; place-items: center; align-content: center; gap: 4px; padding: 6px 2px; border: 0; border-radius: 10px; cursor: pointer; color: #aaa5b3; background: transparent; font-size: 10px; }
.editor-toolbar button i { font-size: 19px; font-style: normal; }.editor-toolbar button:hover, .editor-toolbar button.active { color: #fff; background: #7c3aed; }
.editor-stage-wrap { min-width: 0; min-height: 0; overflow: hidden; }
.editor-inspector { overflow-y: auto; border-left: 1px solid rgba(255,255,255,.08); color: #e7e3ed; background: #24252b; }
.editor-inspector section { padding: 18px 16px; border-bottom: 1px solid rgba(255,255,255,.08); }.editor-inspector section > header { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-bottom: 16px; }
.editor-inspector h2 { margin: 0 0 3px; font-size: 14px; }.editor-inspector header p, .quality-note, .ai-controls p { margin: 0; color: #918c9a; font-size: 10px; line-height: 1.55; }
.field-grid { display: grid; gap: 9px; }.four-fields { grid-template-columns: 1fr 1fr; }.dimension-fields { grid-template-columns: 1fr auto 1fr; align-items: end; }.dimension-fields > span { padding-bottom: 9px; color: #76717f; }
.field-grid label, .select-field, .ai-controls label { display: grid; gap: 6px; color: #aaa5b3; font-size: 10px; }
.field-grid input { min-width: 0; height: 34px; padding: 0 9px; border: 1px solid rgba(255,255,255,.12); border-radius: 7px; outline: none; color: #fff; background: #191a1f; }.field-grid input:focus { border-color: #8b5cf6; box-shadow: 0 0 0 2px rgba(139,92,246,.16); }
.field-error { margin: 8px 0 0; color: #fca5a5; font-size: 10px; }.switch-row { display: flex; align-items: center; justify-content: space-between; margin-top: 12px; color: #aaa5b3; font-size: 11px; }
.preset-grid { display: grid; grid-template-columns: repeat(5, 1fr); gap: 5px; margin-top: 12px; }.preset-grid button, .background-options button, .platform-presets button { min-height: 30px; padding: 4px; border: 1px solid rgba(255,255,255,.1); border-radius: 7px; cursor: pointer; color: #c7c2ce; background: #1b1c21; font-size: 10px; }.preset-grid button:hover, .background-options button.active { border-color: #8b5cf6; color: #fff; background: rgba(124,58,237,.25); }
.platform-presets { margin-top: 12px; color: #aaa5b3; font-size: 10px; }.platform-presets summary { cursor: pointer; }.platform-presets div { display: grid; gap: 6px; margin-top: 8px; }.platform-presets button { text-align: left; }
.swap-button { border: 0; cursor: pointer; color: #c4b5fd; background: transparent; font-size: 18px; }.select-field { margin-top: 12px; }.transform-actions { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; margin-top: 12px; }.quality-note { margin-top: 12px; padding: 10px; border-radius: 8px; color: #c4b5fd; background: rgba(124,58,237,.12); }
.background-options { display: grid; grid-template-columns: repeat(3, 1fr); gap: 7px; }.color-field, .range-field { display: flex; align-items: center; justify-content: space-between; gap: 10px; margin-top: 16px; color: #aaa5b3; font-size: 11px; }.color-field input { width: 46px; height: 32px; padding: 2px; border: 1px solid rgba(255,255,255,.12); border-radius: 6px; background: transparent; }.range-field input { min-width: 0; flex: 1; }
.ai-controls { display: grid; gap: 14px; }.capability-empty { padding: 16px; border: 1px dashed rgba(255,255,255,.15); border-radius: 10px; color: #928d9c; font-size: 11px; line-height: 1.6; }
.export-result { display: grid; gap: 6px; color: #86efac; font-size: 11px; }.export-result span { color: #aaa5b3; }.export-result a { color: #c4b5fd; }
.editor-statusbar { display: flex; align-items: center; gap: 18px; overflow-x: auto; padding: 0 14px; border-top: 1px solid rgba(255,255,255,.08); color: #8f8a98; background: #202126; font-size: 9px; white-space: nowrap; }
@media (max-width: 1050px) { .editor-topbar { grid-template-columns: 1fr auto; }.editor-history-actions { display: none; }.editor-export-actions { min-width: 0; }.editor-export-actions > span, .editor-export-actions > .n-button:nth-of-type(1) { display: none; }.editor-body { grid-template-columns: 64px minmax(0,1fr) 290px; } }
@media (max-width: 760px) { .image-editor-page { grid-template-rows: auto minmax(0,1fr) 28px; }.editor-topbar { min-height: 110px; grid-template-columns: 1fr; padding-block: 8px; }.editor-export-actions { justify-content: flex-start; flex-wrap: wrap; }.editor-body { position: relative; grid-template-columns: 1fr; grid-template-rows: minmax(380px,1fr) auto; overflow-y: auto; }.editor-toolbar { position: fixed; z-index: 8; right: 8px; bottom: 38px; left: 8px; height: 58px; align-items: center; flex-direction: row; justify-content: space-around; padding: 5px; border: 1px solid rgba(255,255,255,.1); border-radius: 14px; box-shadow: 0 8px 30px #0008; }.editor-toolbar button { min-width: 54px; min-height: 46px; }.editor-toolbar button i { font-size: 15px; }.editor-stage-wrap { grid-row: 1; min-height: 420px; }.editor-inspector { grid-row: 2; padding-bottom: 82px; border-top: 1px solid rgba(255,255,255,.08); border-left: 0; }.editor-statusbar { gap: 10px; } }
</style>
