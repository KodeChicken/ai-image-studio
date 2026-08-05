<script setup lang="ts">
import { computed } from 'vue'
import { NButton, NModal } from 'naive-ui'
import { RouterLink } from 'vue-router'

export interface PreviewImage {
  id?: string
  contentUrl: string
  label: string
  metadata: string
  mimeType?: string
  width?: number | null
  height?: number | null
}

const props = defineProps<{ show: boolean; image: PreviewImage | null }>()
const emit = defineEmits<{
  'update:show': [value: boolean]
}>()

const downloadName = computed(() => {
  if (!props.image?.id) return 'ai-image-studio-image.png'
  const extension = props.image.mimeType === 'image/jpeg' ? 'jpg' : props.image.mimeType === 'image/webp' ? 'webp' : 'png'
  return `ai-image-studio-${props.image.id}.${extension}`
})
</script>

<template>
  <n-modal :show="show" @update:show="emit('update:show', $event)">
    <article v-if="image" class="image-preview-dialog" role="dialog" :aria-label="image.label">
      <button type="button" class="preview-close" aria-label="关闭图片预览" @click="emit('update:show', false)">×</button>
      <div class="preview-image-wrap"><img :src="image.contentUrl" :alt="image.label" /></div>
      <footer>
        <div><strong>{{ image.label }}</strong><span>{{ image.metadata }}</span></div>
        <div class="preview-actions">
          <RouterLink v-if="image.id" class="preview-edit-link" :to="`/editor/${image.id}`">编辑成品</RouterLink>
          <n-button tag="a" type="primary" :href="image.contentUrl" :download="downloadName">下载原图</n-button>
        </div>
      </footer>
    </article>
  </n-modal>
</template>

<style scoped>
.image-preview-dialog { position: relative; overflow: hidden; width: min(94vw, 1120px); max-height: 92dvh; border: 1px solid rgba(255,255,255,.12); border-radius: 18px; color: #f6f3fb; background: #17181d; box-shadow: 0 30px 100px #0009; }
.preview-close { position: absolute; z-index: 2; top: 12px; right: 12px; display: grid; width: 36px; height: 36px; place-items: center; padding: 0 0 3px; border: 1px solid rgba(255,255,255,.15); border-radius: 50%; cursor: pointer; color: #fff; background: rgba(12,13,17,.72); font-size: 24px; }
.preview-image-wrap { display: grid; min-height: 300px; max-height: calc(92dvh - 84px); place-items: center; overflow: auto; background: #101116; }
.preview-image-wrap img { display: block; max-width: 100%; max-height: calc(92dvh - 84px); object-fit: contain; }
.image-preview-dialog footer { display: flex; align-items: center; justify-content: space-between; gap: 18px; padding: 12px 16px; }
.image-preview-dialog footer > div:first-child { display: grid; min-width: 0; gap: 3px; }.image-preview-dialog strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }.image-preview-dialog span { color: #9993a2; font-size: 10px; }
.preview-actions { display: flex; flex: 0 0 auto; gap: 8px; }
.preview-edit-link { display: inline-flex; min-height: 34px; align-items: center; padding: 0 14px; border: 1px solid rgba(255,255,255,.15); border-radius: 7px; color: #eeeaf7; text-decoration: none; }
@media (max-width: 640px) { .image-preview-dialog { width: calc(100vw - 16px); max-height: calc(100dvh - 16px); }.image-preview-dialog footer { align-items: stretch; flex-direction: column; }.preview-actions { justify-content: flex-end; } }
</style>
