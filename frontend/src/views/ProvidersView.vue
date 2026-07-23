<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { NButton, NInput, NModal, NPopconfirm, NSelect, NSwitch, useMessage } from 'naive-ui'
import { api } from '@/api/client'
import { useAuthStore } from '@/stores/auth'
import type { ImageModel, Provider } from '@/types/api'

interface ModelPrice {
  id: string
  price: string
  currency: string
  effectiveFrom: string
  effectiveTo: string | null
}

interface TestGenerationResult {
  modelId: string
  modelName: string
  imageDataUrl: string
  mimeType: string
  width: number
  height: number
  latencyMs: number
}

const defaultTestPrompt = 'A cinematic studio photograph of a translucent purple glass sculpture on a clean white pedestal, soft diffused lighting, high detail.'

const providers = ref<Provider[]>([])
const models = ref<ImageModel[]>([])
const dialogOpen = ref(false)
const editing = ref<Provider | null>(null)
const providerKey = ref('openai')
const providerType = ref('openai-compatible')
const displayName = ref('OpenAI Compatible')
const baseUrl = ref('https://api.openai.com/v1')
const apiKey = ref('')
const saving = ref(false)
const refreshingId = ref<string | null>(null)
const testOpen = ref(false)
const testProvider = ref<Provider | null>(null)
const testModelId = ref<string | null>(null)
const testPrompt = ref(defaultTestPrompt)
const testSize = ref('auto')
const testQuality = ref('auto')
const testAspectRatio = ref('auto')
const testingGeneration = ref(false)
const testResult = ref<TestGenerationResult | null>(null)
const pricingOpen = ref(false)
const pricingProvider = ref<Provider | null>(null)
const pricingModel = ref<ImageModel | null>(null)
const prices = ref<ModelPrice[]>([])
const priceValue = ref('')
const priceCurrency = ref('USD')
const pricingSaving = ref(false)
const message = useMessage()
const auth = useAuthStore()

const providerPresets = {
  'openai-compatible': { key: 'openai', name: 'OpenAI Compatible', baseUrl: 'https://api.openai.com/v1' },
  gemini: { key: 'google-gemini', name: 'Google Gemini', baseUrl: 'https://generativelanguage.googleapis.com' },
  grok: { key: 'xai-grok', name: 'xAI Grok', baseUrl: 'https://api.x.ai' },
} as const

const isAdmin = computed(() => auth.user?.role === 'admin')
const imageModels = computed(() => models.value.filter((model) => model.enabled && model.capabilities.text_to_image === true))
const groupedModels = computed(() => new Map(providers.value.map((provider) => [provider.id, imageModels.value.filter((model) => model.providerId === provider.id)])))
const selectedTestModel = computed(() => imageModels.value.find((model) => model.id === testModelId.value) ?? null)
const testModelOptions = computed(() => (testProvider.value ? (groupedModels.value.get(testProvider.value.id) ?? []).map((model) => ({ label: model.displayName, value: model.id })) : []))
const testSizeOptions = computed(() => enumOptions('size'))
const testQualityOptions = computed(() => enumOptions('quality'))
const testAspectRatioOptions = computed(() => enumOptions('aspect_ratio'))

onMounted(load)

watch(providerType, (value) => {
  if (editing.value) return
  const preset = providerPresets[value as keyof typeof providerPresets]
  if (!preset) return
  providerKey.value = preset.key
  displayName.value = preset.name
  baseUrl.value = preset.baseUrl
})

watch(testModelId, resetTestParameters)

async function load() {
  ;[providers.value, models.value] = await Promise.all([
    api<Provider[]>('/api/v1/providers'),
    api<ImageModel[]>('/api/v1/models?includeDiscovered=true&imageOnly=true'),
  ])
}

function openCreate() {
  editing.value = null
  providerKey.value = 'openai'
  providerType.value = 'openai-compatible'
  displayName.value = 'OpenAI Compatible'
  baseUrl.value = 'https://api.openai.com/v1'
  apiKey.value = ''
  dialogOpen.value = true
}

function openEdit(item: Provider) {
  editing.value = item
  providerKey.value = item.providerKey
  providerType.value = item.providerType
  displayName.value = item.displayName
  baseUrl.value = item.baseUrl
  apiKey.value = ''
  dialogOpen.value = true
}

async function save() {
  saving.value = true
  try {
    if (editing.value) {
      await api(`/api/v1/providers/${editing.value.id}`, {
        method: 'PATCH',
        body: JSON.stringify({ displayName: displayName.value, baseUrl: baseUrl.value, apiKey: apiKey.value || undefined }),
      })
    } else {
      await api('/api/v1/providers', {
        method: 'POST',
        body: JSON.stringify({ providerKey: providerKey.value, providerType: providerType.value, displayName: displayName.value, baseUrl: baseUrl.value, apiKey: apiKey.value, config: {} }),
      })
    }
    dialogOpen.value = false
    await load()
    message.success('Provider 已保存')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '保存失败')
  } finally {
    saving.value = false
  }
}

async function toggle(item: Provider, enabled: boolean) {
  await api(`/api/v1/providers/${item.id}`, { method: 'PATCH', body: JSON.stringify({ enabled }) })
  item.enabled = enabled
}

async function refreshModels(item: Provider) {
  refreshingId.value = item.id
  try {
    const result = await api<{ discovered: number; verifiedImageModels: number }>(`/api/v1/providers/${item.id}/models/discover`, { method: 'POST' })
    message.success(`发现 ${result.discovered} 个模型，其中 ${result.verifiedImageModels} 个已识别为生图模型`)
    await load()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '刷新模型列表失败')
  } finally {
    refreshingId.value = null
  }
}

function enumOptions(name: string) {
  const definition = selectedTestModel.value?.parameterSchema.parameters?.[name]
  if (definition?.type !== 'enum') return []
  return (definition.options ?? []).map((value) => ({ label: value === 'auto' ? 'Auto' : value, value }))
}

function enumDefault(name: string, options: Array<{ value: string }>) {
  const value = selectedTestModel.value?.parameterSchema.parameters?.[name]?.default
  return typeof value === 'string' && options.some((option) => option.value === value)
    ? value
    : options[0]?.value ?? 'auto'
}

function resetTestParameters() {
  testSize.value = enumDefault('size', testSizeOptions.value)
  testQuality.value = enumDefault('quality', testQualityOptions.value)
  testAspectRatio.value = enumDefault('aspect_ratio', testAspectRatioOptions.value)
  testResult.value = null
}

function openConnectionTest(item: Provider) {
  testProvider.value = item
  testModelId.value = groupedModels.value.get(item.id)?.[0]?.id ?? null
  testPrompt.value = defaultTestPrompt
  testResult.value = null
  resetTestParameters()
  testOpen.value = true
}

async function runConnectionTest() {
  if (!testProvider.value || !testModelId.value || !testPrompt.value.trim()) return
  const parameters: Record<string, unknown> = { n: 1 }
  if (testSizeOptions.value.length) parameters.size = testSize.value
  if (testQualityOptions.value.length) parameters.quality = testQuality.value
  if (testAspectRatioOptions.value.length) parameters.aspect_ratio = testAspectRatio.value
  testingGeneration.value = true
  try {
    testResult.value = await api<TestGenerationResult>(`/api/v1/providers/${testProvider.value.id}/test-generation`, {
      method: 'POST',
      body: JSON.stringify({ modelId: testModelId.value, prompt: testPrompt.value.trim(), parameters }),
    })
    message.success(`测试图片生成成功，耗时 ${testResult.value.latencyMs} ms`)
    await load()
  } catch (error) {
    await load().catch(() => undefined)
    message.error(error instanceof Error ? error.message : '测试生图失败')
  } finally {
    testingGeneration.value = false
  }
}

function healthLabel(item: Provider) {
  if (item.healthStatus === 'healthy') return '连接正常'
  if (item.healthStatus === 'unhealthy') return '连接异常'
  return '尚未测试'
}

async function openPricing(provider: Provider, model: ImageModel) {
  pricingProvider.value = provider
  pricingModel.value = model
  priceValue.value = ''
  priceCurrency.value = 'USD'
  prices.value = await api<ModelPrice[]>(`/api/v1/providers/${provider.id}/models/${model.id}/pricing`)
  pricingOpen.value = true
}

async function savePrice() {
  if (!isAdmin.value || !pricingProvider.value || !pricingModel.value) return
  pricingSaving.value = true
  try {
    await api(`/api/v1/providers/${pricingProvider.value.id}/models/${pricingModel.value.id}/pricing`, {
      method: 'POST',
      body: JSON.stringify({ price: priceValue.value, currency: priceCurrency.value }),
    })
    prices.value = await api<ModelPrice[]>(`/api/v1/providers/${pricingProvider.value.id}/models/${pricingModel.value.id}/pricing`)
    priceValue.value = ''
    message.success('模型单张图片价格已保存')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '价格保存失败')
  } finally {
    pricingSaving.value = false
  }
}

async function removePrice(item: ModelPrice) {
  if (!isAdmin.value || !pricingProvider.value || !pricingModel.value) return
  await api(`/api/v1/providers/${pricingProvider.value.id}/models/${pricingModel.value.id}/pricing/${item.id}`, { method: 'DELETE' })
  prices.value = prices.value.filter((price) => price.id !== item.id)
  message.success('价格记录已删除')
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div><span class="eyebrow muted">MODEL CONNECTIONS</span><h1>我的 Provider</h1><p>每个用户独立管理自己的上游地址、凭据与模型。</p></div>
      <n-button type="primary" @click="openCreate">添加 Provider</n-button>
    </header>
    <div class="provider-grid">
      <article v-for="item in providers" :key="item.id" class="provider-card panel">
        <header>
          <div class="provider-logo">{{ item.displayName.slice(0, 2).toUpperCase() }}</div>
          <div><h2>{{ item.displayName }}</h2><p>{{ item.providerType }}</p></div>
          <n-switch :value="item.enabled" @update:value="value => toggle(item, value)" />
        </header>
        <dl>
          <div><dt>Base URL</dt><dd>{{ item.baseUrl }}</dd></div>
          <div><dt>凭据</dt><dd>{{ item.credentialConfigured ? '已安全配置' : '未配置' }}</dd></div>
          <div><dt>连接状态</dt><dd :class="['provider-health', item.healthStatus]" :title="item.lastHealthError || undefined">{{ healthLabel(item) }}</dd></div>
        </dl>
        <div class="model-tags">
          <span v-for="model in groupedModels.get(item.id)" :key="model.id" :class="model.availabilityStatus">
            {{ model.displayName }} · 可生图
            <n-button text size="tiny" @click="openPricing(item, model)">价格</n-button>
          </span>
          <small v-if="!groupedModels.get(item.id)?.length">尚未发现可用生图模型，请先刷新模型列表</small>
        </div>
        <footer>
          <n-button @click="openEdit(item)">编辑配置</n-button>
          <n-button @click="openConnectionTest(item)">测试连接</n-button>
          <n-button :loading="refreshingId === item.id" @click="refreshModels(item)">刷新模型列表</n-button>
        </footer>
      </article>
      <button v-if="!providers.length" class="add-provider-card panel" @click="openCreate">＋<strong>添加第一个 Provider</strong><span>配置后即可刷新模型列表</span></button>
    </div>
  </div>
  <n-modal v-model:show="dialogOpen" preset="card" :title="editing ? '编辑 Provider' : '添加 Provider'" class="dialog-card provider-dialog">
    <div class="form-stack">
      <p class="modal-intro">支持任意 OpenAI Compatible 供应商地址，也可选择 Gemini 或 Grok 原生协议。配置仅对当前用户生效。</p>
      <label>配置标识<n-input v-model:value="providerKey" :disabled="Boolean(editing)" placeholder="例如 openai-main" /></label>
      <label>协议类型<n-select v-model:value="providerType" :disabled="Boolean(editing)" :options="[{ label: 'OpenAI Compatible（自定义供应商）', value: 'openai-compatible' }, { label: 'Gemini', value: 'gemini' }, { label: 'Grok', value: 'grok' }]" /></label>
      <label>显示名称<n-input v-model:value="displayName" /></label>
      <label>Base URL<n-input v-model:value="baseUrl" placeholder="https://example.com/v1" /></label>
      <label>API Key<n-input v-model:value="apiKey" type="password" show-password-on="click" :placeholder="editing ? '留空表示不轮换凭据' : '仅写入，不会回显'" /></label>
      <n-button type="primary" :loading="saving" @click="save">保存配置</n-button>
    </div>
  </n-modal>
  <n-modal v-model:show="testOpen" preset="card" title="测试生图连接" class="dialog-card connection-test-dialog">
    <div class="form-stack">
      <p class="modal-intro">选择已识别的生图模型，使用可编辑提示词和一组最小参数发起真实生图请求。测试可能产生上游费用。</p>
      <div :class="['connection-test-layout', { 'with-result': testResult }]">
        <div class="test-controls">
          <label>生图模型<n-select v-model:value="testModelId" :options="testModelOptions" placeholder="暂无可测试的生图模型" /></label>
          <label>测试提示词<n-input v-model:value="testPrompt" type="textarea" :autosize="{ minRows: 3, maxRows: 6 }" maxlength="4000" show-count /></label>
          <div v-if="selectedTestModel" class="test-parameter-grid">
            <label v-if="testSizeOptions.length">尺寸<n-select v-model:value="testSize" :options="testSizeOptions" /></label>
            <label v-if="testQualityOptions.length">质量<n-select v-model:value="testQuality" :options="testQualityOptions" /></label>
            <label v-if="testAspectRatioOptions.length">宽高比<n-select v-model:value="testAspectRatio" :options="testAspectRatioOptions" /></label>
            <label>数量<n-input value="1" disabled /></label>
          </div>
          <div v-else class="test-empty">当前 Provider 尚未发现生图模型，请关闭窗口并先刷新模型列表。</div>
          <n-button type="primary" :loading="testingGeneration" :disabled="!testModelId || !testPrompt.trim()" @click="runConnectionTest">生成测试图片</n-button>
        </div>
        <figure v-if="testResult" class="test-result">
          <img :src="testResult.imageDataUrl" :alt="`${testResult.modelName} 测试生成结果`">
          <figcaption>
            <strong>{{ testResult.modelName }}</strong>
            <span>{{ testResult.width }} × {{ testResult.height }} · {{ testResult.mimeType }} · {{ testResult.latencyMs }} ms</span>
          </figcaption>
        </figure>
      </div>
    </div>
  </n-modal>
  <n-modal v-model:show="pricingOpen" preset="card" :title="`${pricingModel?.displayName || ''} · 价格`" class="dialog-card">
    <div class="form-stack">
      <p class="muted">配置平台成本估算使用的单张图片价格；这不是上游最终账单。</p>
      <div v-if="prices.length" class="pricing-list">
        <div v-for="item in prices" :key="item.id">
          <span>{{ item.currency }} {{ item.price }} / 张</span>
          <small>{{ new Date(item.effectiveFrom).toLocaleString() }} 起</small>
          <n-popconfirm v-if="isAdmin" @positive-click="removePrice(item)">
            <template #trigger><n-button text type="error" size="small">删除</n-button></template>
            删除后新的生图任务将不再使用这条价格，历史费用快照不受影响。
          </n-popconfirm>
        </div>
      </div>
      <p v-else class="muted">暂未配置价格，任务仍会记录用量，但成本为空。</p>
      <template v-if="isAdmin">
        <label>单张价格<n-input v-model:value="priceValue" placeholder="例如 0.04" /></label>
        <label>币种<n-input v-model:value="priceCurrency" placeholder="USD" /></label>
        <n-button type="primary" :loading="pricingSaving" :disabled="!priceValue.trim()" @click="savePrice">保存当前价格</n-button>
      </template>
      <p v-else class="pricing-readonly">模型价格由管理员统一维护，普通用户仅可查看。</p>
    </div>
  </n-modal>
</template>
