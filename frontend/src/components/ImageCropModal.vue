<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { NButton, NInputNumber, NModal, NSlider, useMessage } from 'naive-ui'
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
const zoom = ref(1)
const minZoom = ref(0.1)
const maxZoom = ref(4)
const exporting = ref(false)
let cropper: Cropper | null = null

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

watch([outputWidth, outputHeight], () => {
  if (cropper && validOutput.value) {
    cropper.setAspectRatio(outputWidth.value! / outputHeight.value!)
  }
})

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
    aspectRatio: outputWidth.value! / outputHeight.value!,
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
    cropBoxMovable: false,
    cropBoxResizable: false,
    toggleDragModeOnDblclick: false,
    ready() {
      if (!cropper) return
      const imageData = cropper.getImageData()
      const initialZoom = imageData.naturalWidth > 0 ? imageData.width / imageData.naturalWidth : 1
      minZoom.value = initialZoom
      maxZoom.value = Math.max(initialZoom * 4, initialZoom + 1)
      zoom.value = initialZoom
    },
    zoom(event) {
      zoom.value = event.detail.ratio
    },
  })
}

function setZoom(value: number) {
  zoom.value = value
  cropper?.zoomTo(value)
}

function resetCrop() {
  cropper?.reset()
  if (!cropper) return
  const imageData = cropper.getImageData()
  const initialZoom = imageData.naturalWidth > 0 ? imageData.width / imageData.naturalWidth : minZoom.value
  zoom.value = initialZoom
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
          <span>{{ cropMode ? `输出比例 ${outputWidth || '—'}:${outputHeight || '—'}` : image.metadata }}</span>
        </div>
        <button type="button" aria-label="关闭图片预览" @click="close">×</button>
      </header>

      <div v-if="cropMode" class="image-crop-workspace">
        <div class="image-crop-canvas">
          <img ref="cropImage" :src="image.contentUrl" :alt="image.label" @load="initializeCropper" />
        </div>
        <aside class="image-crop-controls">
          <div class="image-crop-size-fields">
            <label>输出宽度<n-input-number v-model:value="outputWidth" :min="16" :max="8192" :precision="0" /></label>
            <label>输出高度<n-input-number v-model:value="outputHeight" :min="16" :max="8192" :precision="0" /></label>
          </div>
          <small>拖动画面调整位置，滚轮或双指可缩放。导出的文件不会覆盖原图。</small>
          <label class="image-crop-zoom">
            缩放
            <n-slider :value="zoom" :min="minZoom" :max="maxZoom" :step="0.01" @update:value="setZoom" />
          </label>
          <div class="image-crop-actions">
            <n-button @click="resetCrop">重置</n-button>
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
