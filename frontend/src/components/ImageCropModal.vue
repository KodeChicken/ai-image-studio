<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { NButton, NModal, useMessage } from 'naive-ui'
import Cropper from 'cropperjs'
import 'cropperjs/dist/cropper.css'

export interface CropPreviewImage {
  id?: string
  contentUrl: string
  label: string
  metadata: string
  mimeType?: string
  width?: number | null
  height?: number | null
}

const props = defineProps<{
  show: boolean
  image: CropPreviewImage | null
  initialMode?: 'preview' | 'crop'
}>()
const emit = defineEmits<{ 'update:show': [value: boolean] }>()
const message = useMessage()
const cropImage = ref<HTMLImageElement | null>(null)
const cropMode = ref(false)
const outputWidth = ref<number | null>(null)
const outputHeight = ref<number | null>(null)
const exporting = ref(false)
let cropper: Cropper | null = null
let minZoomRatio = 0.01
let maxZoomRatio = 8

const canCrop = computed(() => Boolean(
  props.image?.id
  && props.image.width
  && props.image.height
  && props.image.mimeType,
))
const validOutput = computed(() => {
  const width = outputWidth.value
  const height = outputHeight.value
  return width !== null
    && height !== null
    && Number.isInteger(width)
    && Number.isInteger(height)
    && width >= 16
    && height >= 16
    && width <= 8192
    && height <= 8192
    && width * height <= 33_554_432
})

watch(
  [() => props.show, () => props.image?.contentUrl, () => props.initialMode],
  async ([show]) => {
    leaveCropMode()
    if (show && props.initialMode === 'crop') await enterCropMode()
  },
)

onBeforeUnmount(destroyCropper)

async function enterCropMode() {
  if (!props.image || !canCrop.value) return
  outputWidth.value = props.image.width ?? null
  outputHeight.value = props.image.height ?? null
  cropMode.value = true
  await nextTick()
  if (cropImage.value?.complete) initializeCropper()
}

function initializeCropper() {
  if (!cropImage.value || !validOutput.value) return
  destroyCropper()
  cropper = new Cropper(cropImage.value, {
    viewMode: 0,
    dragMode: 'move',
    autoCropArea: 0.82,
    background: false,
    center: true,
    guides: true,
    movable: true,
    zoomable: true,
    zoomOnTouch: true,
    zoomOnWheel: true,
    cropBoxMovable: true,
    cropBoxResizable: true,
    minCropBoxWidth: 48,
    minCropBoxHeight: 48,
    toggleDragModeOnDblclick: false,
    ready() {
      if (!cropper) return
      const imageData = cropper.getImageData()
      const initialZoom = imageData.naturalWidth > 0 ? imageData.width / imageData.naturalWidth : 1
      minZoomRatio = Math.max(initialZoom * 0.25, 0.01)
      maxZoomRatio = Math.max(initialZoom * 8, initialZoom + 2)
      const cropData = cropper.getData(true)
      updateCropDimensions(cropData.width, cropData.height)
    },
    crop(event) {
      updateCropDimensions(event.detail.width, event.detail.height)
    },
    zoom(event) {
      if (event.detail.ratio < minZoomRatio || event.detail.ratio > maxZoomRatio) {
        event.preventDefault()
      }
    },
  })
}

function updateCropDimensions(width: number, height: number) {
  const cropBox = cropImage.value
    ?.parentElement
    ?.querySelector<HTMLElement>('.cropper-crop-box')
  if (cropBox) cropBox.dataset.cropDimensions = `${Math.round(width)} × ${Math.round(height)} px`
}

function updateOutputDimension(edge: 'width' | 'height', event: Event) {
  const input = event.target as HTMLInputElement
  const digits = input.value.replace(/\D/g, '')
  if (input.value !== digits) input.value = digits
  const value = digits ? Number(digits) : null
  if (edge === 'width') outputWidth.value = value
  else outputHeight.value = value
}

function selectDimension(event: FocusEvent) {
  const input = event.target as HTMLInputElement
  input.select()
}

function leaveCropMode() {
  destroyCropper()
  cropMode.value = false
  exporting.value = false
}

function destroyCropper() {
  cropper?.destroy()
  cropper = null
}

function close() {
  emit('update:show', false)
}

async function exportCrop() {
  if (!cropper || !props.image || !validOutput.value) return
  exporting.value = true
  try {
    const mimeType = supportedOutputMime(props.image.mimeType)
    const canvas = cropper.getCroppedCanvas({
      width: outputWidth.value!,
      height: outputHeight.value!,
      imageSmoothingEnabled: true,
      imageSmoothingQuality: 'high',
      fillColor: mimeType === 'image/jpeg' ? '#ffffff' : undefined,
    })
    const blob = await canvasBlob(canvas, mimeType)
    downloadBlob(blob, cropDownloadName(props.image, mimeType, outputWidth.value!, outputHeight.value!))
    message.success(`已导出 ${outputWidth.value} × ${outputHeight.value}`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '裁剪结果导出失败')
  } finally {
    exporting.value = false
  }
}

function supportedOutputMime(value?: string) {
  return value === 'image/jpeg' || value === 'image/webp' ? value : 'image/png'
}

function canvasBlob(canvas: HTMLCanvasElement, mimeType: string): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => blob ? resolve(blob) : reject(new Error('浏览器无法生成裁剪图片')),
      mimeType,
      mimeType === 'image/png' ? undefined : 0.95,
    )
  })
}

function cropDownloadName(image: CropPreviewImage, mimeType: string, width: number, height: number) {
  const extension = mimeType === 'image/jpeg' ? 'jpg' : mimeType === 'image/webp' ? 'webp' : 'png'
  return `ai-image-studio-${image.id ?? 'crop'}-${width}x${height}.${extension}`
}

function originalDownloadName(image: CropPreviewImage) {
  const mimeType = supportedOutputMime(image.mimeType)
  const extension = mimeType === 'image/jpeg' ? 'jpg' : mimeType === 'image/webp' ? 'webp' : 'png'
  return `ai-image-studio-${image.id ?? 'image'}.${extension}`
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}
</script>

<template>
  <n-modal :show="show" :mask-closable="!cropMode" @update:show="emit('update:show', $event)">
    <section v-if="image" class="image-editor-modal" role="dialog" aria-modal="true" :aria-label="image.label">
      <header>
        <div>
          <strong>{{ cropMode ? '裁剪与缩放' : image.label }}</strong>
          <span>{{ cropMode ? `输出尺寸 ${outputWidth || '—'} × ${outputHeight || '—'}` : image.metadata }}</span>
        </div>
        <button type="button" aria-label="关闭图片预览" @click="close">×</button>
      </header>

      <div v-if="cropMode" class="image-crop-workspace">
        <div class="image-crop-canvas">
          <img ref="cropImage" :src="image.contentUrl" :alt="image.label" @load="initializeCropper" />
        </div>
        <aside class="image-crop-controls">
          <strong>输出尺寸</strong>
          <div class="image-crop-size-fields">
            <label>输出宽度<input :value="outputWidth ?? ''" type="text" inputmode="numeric" pattern="[0-9]*" aria-label="输出宽度" @input="updateOutputDimension('width', $event)" @focus="selectDimension" /></label>
            <label>输出高度<input :value="outputHeight ?? ''" type="text" inputmode="numeric" pattern="[0-9]*" aria-label="输出高度" @input="updateOutputDimension('height', $event)" @focus="selectDimension" /></label>
          </div>
          <small v-if="!validOutput" class="image-crop-size-error">宽高需为 16-8192 的整数，且总像素不能超过 3355 万。</small>
          <div class="image-crop-actions">
            <n-button @click="leaveCropMode">返回预览</n-button>
            <n-button type="primary" :loading="exporting" :disabled="!validOutput" @click="exportCrop">导出成品</n-button>
          </div>
        </aside>
      </div>

      <template v-else>
        <div class="image-preview-stage">
          <img :src="image.contentUrl" :alt="image.label" />
        </div>
        <footer>
          <span>{{ image.metadata }}</span>
          <div>
            <a v-if="image.id && image.mimeType" :href="image.contentUrl" :download="originalDownloadName(image)">下载原图</a>
            <n-button v-if="canCrop" type="primary" size="small" @click="enterCropMode">裁剪缩放</n-button>
          </div>
        </footer>
      </template>
    </section>
  </n-modal>
</template>
