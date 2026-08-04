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
const sourceWidth = ref<number | null>(null)
const sourceHeight = ref<number | null>(null)
const cropWidth = ref<number | null>(null)
const cropHeight = ref<number | null>(null)
const exporting = ref(false)
let cropper: Cropper | null = null
let minZoomRatio = 0.01
let maxZoomRatio = 8

const canCrop = computed(() => Boolean(
  props.image?.id
  && props.image.mimeType,
))
const validCrop = computed(() => {
  const width = cropWidth.value
  const height = cropHeight.value
  return width !== null
    && height !== null
    && sourceWidth.value !== null
    && sourceHeight.value !== null
    && Number.isInteger(width)
    && Number.isInteger(height)
    && width >= 16
    && height >= 16
    && width <= 8192
    && height <= 8192
    && width <= sourceWidth.value
    && height <= sourceHeight.value
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
  sourceWidth.value = null
  sourceHeight.value = null
  cropWidth.value = null
  cropHeight.value = null
  cropMode.value = true
  await nextTick()
  if (cropImage.value?.complete) initializeCropper()
}

function initializeCropper() {
  if (!cropImage.value) return
  destroyCropper()
  cropper = new Cropper(cropImage.value, {
    viewMode: 1,
    dragMode: 'move',
    autoCropArea: 1,
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
      sourceWidth.value = Math.round(imageData.naturalWidth)
      sourceHeight.value = Math.round(imageData.naturalHeight)
      const initialZoom = imageData.naturalWidth > 0 ? imageData.width / imageData.naturalWidth : 1
      minZoomRatio = Math.max(initialZoom * 0.25, 0.01)
      maxZoomRatio = Math.max(initialZoom * 8, initialZoom + 2)
      cropper.setData({
        x: 0,
        y: 0,
        width: sourceWidth.value,
        height: sourceHeight.value,
      })
      const cropData = cropper.getData(true)
      syncCropDimensions(cropData.width, cropData.height)
    },
    crop(event) {
      syncCropDimensions(event.detail.width, event.detail.height)
    },
    zoom(event) {
      if (event.detail.ratio < minZoomRatio || event.detail.ratio > maxZoomRatio) {
        event.preventDefault()
      }
    },
  })
}

function syncCropDimensions(width: number, height: number) {
  cropWidth.value = Math.round(width)
  cropHeight.value = Math.round(height)
  const cropBox = cropImage.value
    ?.parentElement
    ?.querySelector<HTMLElement>('.cropper-crop-box')
  if (cropBox) cropBox.dataset.cropDimensions = `${cropWidth.value} × ${cropHeight.value} px`
}

function updateCropDimension(edge: 'width' | 'height', event: Event) {
  const input = event.target as HTMLInputElement
  const digits = input.value.replace(/\D/g, '')
  if (input.value !== digits) input.value = digits
  const value = digits ? Number(digits) : null
  if (edge === 'width') cropWidth.value = value
  else cropHeight.value = value
  if (!cropper || value === null || !Number.isInteger(value) || value < 16) return

  const limit = edge === 'width' ? sourceWidth.value : sourceHeight.value
  if (limit === null || value > limit || value > 8192) return
  const cropData = cropper.getData(true)
  if (edge === 'width') {
    const x = Math.max(0, Math.min(cropData.x + (cropData.width - value) / 2, limit - value))
    cropper.setData({ x, width: value })
  } else {
    const y = Math.max(0, Math.min(cropData.y + (cropData.height - value) / 2, limit - value))
    cropper.setData({ y, height: value })
  }
  const updated = cropper.getData(true)
  syncCropDimensions(updated.width, updated.height)
}

function selectDimension(event: FocusEvent) {
  const input = event.target as HTMLInputElement
  input.select()
}

function leaveCropMode() {
  destroyCropper()
  cropMode.value = false
  sourceWidth.value = null
  sourceHeight.value = null
  cropWidth.value = null
  cropHeight.value = null
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
  if (!cropper || !props.image || !validCrop.value) return
  exporting.value = true
  try {
    const mimeType = supportedOutputMime(props.image.mimeType)
    const cropData = cropper.getData(true)
    const width = Math.round(cropData.width)
    const height = Math.round(cropData.height)
    const canvas = cropper.getCroppedCanvas({
      rounded: true,
      imageSmoothingEnabled: true,
      imageSmoothingQuality: 'high',
      fillColor: mimeType === 'image/jpeg' ? '#ffffff' : undefined,
    })
    if (!canvas || canvas.width !== width || canvas.height !== height) {
      throw new Error('裁剪结果尺寸与原图选区不一致')
    }
    const blob = await canvasBlob(canvas, mimeType)
    downloadBlob(blob, cropDownloadName(props.image, mimeType, width, height))
    message.success(`已导出 ${width} × ${height}`)
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
          <span>{{ cropMode ? `原图尺寸 ${sourceWidth || '—'} × ${sourceHeight || '—'}` : image.metadata }}</span>
        </div>
        <button type="button" aria-label="关闭图片预览" @click="close">×</button>
      </header>

      <div v-if="cropMode" class="image-crop-workspace">
        <div class="image-crop-canvas">
          <img ref="cropImage" :src="image.contentUrl" :alt="image.label" @load="initializeCropper" />
        </div>
        <aside class="image-crop-controls">
          <strong>裁剪尺寸（原图像素）</strong>
          <div class="image-crop-size-fields">
            <label>裁剪宽度<input :value="cropWidth ?? ''" type="text" inputmode="numeric" pattern="[0-9]*" aria-label="裁剪宽度" @input="updateCropDimension('width', $event)" @focus="selectDimension" /></label>
            <label>裁剪高度<input :value="cropHeight ?? ''" type="text" inputmode="numeric" pattern="[0-9]*" aria-label="裁剪高度" @input="updateCropDimension('height', $event)" @focus="selectDimension" /></label>
          </div>
          <small v-if="sourceWidth && sourceHeight">当前选区直接截取原图，不会缩放或拉伸。</small>
          <small v-if="sourceWidth && sourceHeight && !validCrop" class="image-crop-size-error">宽高需为 16-8192 的整数、不能超过原图 {{ sourceWidth }} × {{ sourceHeight }}，且总像素不能超过 3355 万。</small>
          <div class="image-crop-actions">
            <n-button @click="leaveCropMode">返回预览</n-button>
            <n-button type="primary" :loading="exporting" :disabled="!validCrop" @click="exportCrop">导出成品</n-button>
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
