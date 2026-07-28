<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { NButton, NInputNumber, NSelect, useMessage } from 'naive-ui'
import { api } from '@/api/client'
import ImageCropModal, { type CropPreviewImage } from '@/components/ImageCropModal.vue'
import type { Conversation, ImageModel, Provider } from '@/types/api'

interface HistoryItem {
  taskId: string
  conversationId: string
  conversationTitle: string
  assetId: string
  contentUrl: string
  modelId: string
  modelName: string
  providerId: string
  providerName: string
  prompt: string
  mimeType: string
  width: number | null
  height: number | null
  fileSizeBytes: number
  createdAt: string
}

const items = ref<HistoryItem[]>([])
const conversations = ref<Conversation[]>([])
const providers = ref<Provider[]>([])
const models = ref<ImageModel[]>([])
const conversationId = ref<string | null>(null)
const providerId = ref<string | null>(null)
const modelId = ref<string | null>(null)
const dateFrom = ref('')
const dateTo = ref('')
const width = ref<number | null>(null)
const height = ref<number | null>(null)
const loading = ref(false)
const message = useMessage()
const imagePreviewOpen = ref(false)
const imagePreview = ref<CropPreviewImage | null>(null)
const modelOptions = computed(() =>
  models.value
    .filter((item) => !providerId.value || item.providerId === providerId.value)
    .map((item) => ({ label: item.displayName, value: item.id })),
)

watch(providerId, (value) => {
  if (modelId.value && !models.value.some((item) => item.id === modelId.value && (!value || item.providerId === value))) {
    modelId.value = null
  }
})

onMounted(async () => {
  ;[conversations.value, providers.value, models.value] = await Promise.all([
    api<Conversation[]>('/api/v1/conversations'),
    api<Provider[]>('/api/v1/providers'),
    api<ImageModel[]>('/api/v1/models?includeDiscovered=true'),
  ])
  await load()
})

async function load() {
  loading.value = true
  try {
    const query = new URLSearchParams()
    if (conversationId.value) query.set('conversationId', conversationId.value)
    if (providerId.value) query.set('providerId', providerId.value)
    if (modelId.value) query.set('modelId', modelId.value)
    if (dateFrom.value) query.set('createdFrom', dateBoundary(dateFrom.value, false))
    if (dateTo.value) query.set('createdTo', dateBoundary(dateTo.value, true))
    if (width.value && height.value) {
      query.set('width', String(width.value))
      query.set('height', String(height.value))
    }
    items.value = await api(`/api/v1/history?${query}`)
  } catch (error) {
    message.error(error instanceof Error ? error.message : '加载历史作品失败')
  } finally {
    loading.value = false
  }
}

async function applyFilters() {
  if (Boolean(width.value) !== Boolean(height.value)) return message.error('宽度和高度需要同时填写')
  if (dateFrom.value && dateTo.value && dateFrom.value > dateTo.value) {
    return message.error('开始日期不能晚于结束日期')
  }
  await load()
}

async function resetFilters() {
  conversationId.value = providerId.value = modelId.value = null
  dateFrom.value = dateTo.value = ''
  width.value = height.value = null
  await load()
}

function dateBoundary(value: string, nextDay: boolean) {
  const [year, month, day] = value.split('-').map(Number)
  const date = new Date(year!, month! - 1, day!)
  if (nextDay) date.setDate(date.getDate() + 1)
  return date.toISOString()
}

function openImagePreview(item: HistoryItem) {
  imagePreview.value = {
    id: item.assetId,
    contentUrl: item.contentUrl,
    label: item.prompt,
    metadata: `${item.providerName} · ${item.modelName} · ${item.width} × ${item.height}`,
    mimeType: item.mimeType,
    width: item.width,
    height: item.height,
  }
  imagePreviewOpen.value = true
}

function imageDownloadName(item: HistoryItem) {
  const extension = item.mimeType === 'image/jpeg' ? 'jpg' : item.mimeType === 'image/webp' ? 'webp' : 'png'
  return `ai-image-studio-${item.assetId}.${extension}`
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div><span class="eyebrow muted">GALLERY</span><h1>历史作品</h1><p>这里展示已经完成持久化的生成结果。</p></div>
      <n-button :loading="loading" @click="load">刷新</n-button>
    </header>
    <div class="filter-bar panel">
      <label class="filter-field"><span>会话</span><n-select v-model:value="conversationId" clearable placeholder="全部会话" :options="conversations.map(item => ({ label: item.title, value: item.id }))" /></label>
      <label class="filter-field"><span>Provider</span><n-select v-model:value="providerId" clearable placeholder="全部 Provider" :options="providers.map(item => ({ label: item.displayName, value: item.id }))" /></label>
      <label class="filter-field"><span>模型</span><n-select v-model:value="modelId" clearable placeholder="全部模型" :options="modelOptions" /></label>
      <label class="filter-field date-range-field"><span>生成日期</span><div><input v-model="dateFrom" type="date" aria-label="开始日期" /><i>至</i><input v-model="dateTo" type="date" aria-label="结束日期" /></div></label>
      <label class="filter-field size-filter"><span>图片尺寸</span><div><n-input-number v-model:value="width" :min="1" :max="100000" placeholder="宽度" /><i>×</i><n-input-number v-model:value="height" :min="1" :max="100000" placeholder="高度" /></div></label>
      <div class="history-filter-actions"><n-button @click="resetFilters">重置</n-button><n-button type="primary" :loading="loading" @click="applyFilters">应用筛选</n-button></div>
    </div>
    <div v-if="items.length" class="history-grid">
      <article v-for="item in items" :key="item.assetId" class="history-card panel">
        <button type="button" class="history-image-button" :aria-label="`放大历史作品：${item.prompt}`" @click="openImagePreview(item)">
          <img :src="item.contentUrl" :alt="item.prompt" />
        </button>
        <div class="history-copy">
          <strong>{{ item.conversationTitle }}</strong>
          <p>{{ item.prompt }}</p>
          <footer class="history-copy-footer">
            <small>{{ item.providerName }} · {{ item.modelName }} · {{ item.width }}×{{ item.height }}</small>
            <a class="image-download-button" :href="item.contentUrl" :download="imageDownloadName(item)">↓ 下载原图</a>
          </footer>
        </div>
      </article>
    </div>
    <div v-else class="large-empty panel"><span>▦</span><h2>还没有历史作品</h2><p>完成的生图会自动出现在这里。</p></div>
  </div>

  <image-crop-modal v-model:show="imagePreviewOpen" :image="imagePreview" />
</template>
