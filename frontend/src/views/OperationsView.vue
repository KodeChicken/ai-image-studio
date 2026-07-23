<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { NButton, NInput, NSpin, useMessage } from 'naive-ui'
import { api } from '@/api/client'

interface Analytics {
  totals: { totalTasks: number; succeededTasks: number; failedTasks: number; activeTasks: number; retryCount: number; generatedImages: number; successRate: number; p50LatencyMs: number | null; p95LatencyMs: number | null; p99LatencyMs: number | null }
  providers: Array<{ providerId: string; providerName: string; providerType: string; taskCount: number; succeededTasks: number; failedTasks: number; averageLatencyMs: number | null }>
  daily: Array<{ day: string; taskCount: number; succeededTasks: number; failedTasks: number; imageCount: number }>
  storage: Array<{ driver: string; assetCount: number; fileSizeBytes: number }>
  costs: Array<{ currency: string; totalCost: number }>
}
interface RequestLog { id: number; taskId: string | null; traceId: string; route: string; method: string; providerType: string | null; modelKey: string | null; statusCode: number | null; latencyMs: number | null; errorCode: string | null; errorSummary: string | null; createdAt: string }
interface LogPage { items: RequestLog[]; nextBeforeId: number | null }

const analytics = ref<Analytics | null>(null)
const logs = ref<RequestLog[]>([])
const nextBeforeId = ref<number | null>(null)
const traceId = ref('')
const loading = ref(false)
const loadingMore = ref(false)
const message = useMessage()
const successRate = computed(() => `${((analytics.value?.totals.successRate || 0) * 100).toFixed(1)}%`)
const costSummary = computed(() => analytics.value?.costs.map(item => `${item.currency} ${item.totalCost.toFixed(4)}`).join(' · ') || '未配置价格')

onMounted(load)

async function load() {
  loading.value = true
  try {
    const [metrics, page] = await Promise.all([api<Analytics>('/api/v1/admin/analytics'), loadLogsPage()])
    analytics.value = metrics
    logs.value = page.items
    nextBeforeId.value = page.nextBeforeId
  } catch (error) {
    message.error(error instanceof Error ? error.message : '运营数据加载失败')
  } finally {
    loading.value = false
  }
}

async function loadLogsPage(beforeId?: number) {
  const query = new URLSearchParams({ limit: '50' })
  if (traceId.value.trim()) query.set('traceId', traceId.value.trim())
  if (beforeId) query.set('beforeId', String(beforeId))
  return api<LogPage>(`/api/v1/admin/request-logs?${query}`)
}

async function filterLogs() {
  loading.value = true
  try {
    const page = await loadLogsPage()
    logs.value = page.items
    nextBeforeId.value = page.nextBeforeId
  } catch (error) {
    message.error(error instanceof Error ? error.message : '日志加载失败')
  } finally {
    loading.value = false
  }
}

async function loadMore() {
  if (!nextBeforeId.value) return
  loadingMore.value = true
  try {
    const page = await loadLogsPage(nextBeforeId.value)
    logs.value.push(...page.items)
    nextBeforeId.value = page.nextBeforeId
  } finally {
    loadingMore.value = false
  }
}

function duration(value: number | null) { return value == null ? '—' : `${Math.round(value)} ms` }
function fileSize(value: number) { return value < 1024 ** 2 ? `${(value / 1024).toFixed(1)} KB` : `${(value / 1024 ** 2).toFixed(1)} MB` }
function dateTime(value: string) { return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'medium' }).format(new Date(value)) }
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div><span class="eyebrow muted">ADMIN OBSERVABILITY</span><h1>运营监控</h1><p>最近 30 天的任务、Provider、存储与脱敏请求日志。</p></div>
      <span class="admin-badge">仅管理员可见</span>
    </header>
    <n-spin :show="loading">
      <div v-if="analytics" class="metric-grid admin-metrics">
        <article class="panel metric-card"><small>任务总数</small><strong>{{ analytics.totals.totalTasks }}</strong><span>{{ analytics.totals.activeTasks }} 个处理中</span></article>
        <article class="panel metric-card"><small>成功率</small><strong>{{ successRate }}</strong><span>{{ analytics.totals.failedTasks }} 个失败</span></article>
        <article class="panel metric-card"><small>P95 耗时</small><strong>{{ duration(analytics.totals.p95LatencyMs) }}</strong><span>P50 {{ duration(analytics.totals.p50LatencyMs) }} · P99 {{ duration(analytics.totals.p99LatencyMs) }}</span></article>
        <article class="panel metric-card"><small>图片 / 重试</small><strong>{{ analytics.totals.generatedImages }}</strong><span>{{ analytics.totals.retryCount }} 次重试</span></article>
        <article class="panel metric-card wide-metric"><small>已计算成本</small><strong class="cost-copy">{{ costSummary }}</strong><span>不同币种分别汇总</span></article>
      </div>
      <div class="analytics-columns">
        <section class="analytics-section"><header><div><h2>Provider 表现</h2><p>成功/失败量和平均任务耗时。</p></div></header><div class="panel table-panel"><table><thead><tr><th>Provider</th><th>任务</th><th>成功</th><th>失败</th><th>平均耗时</th></tr></thead><tbody><tr v-for="item in analytics?.providers" :key="item.providerId"><td><strong>{{ item.providerName }}</strong><small>{{ item.providerType }}</small></td><td>{{ item.taskCount }}</td><td>{{ item.succeededTasks }}</td><td>{{ item.failedTasks }}</td><td>{{ duration(item.averageLatencyMs) }}</td></tr><tr v-if="!analytics?.providers.length"><td colspan="5" class="muted-cell">暂无任务数据</td></tr></tbody></table></div></section>
        <section class="analytics-section"><header><div><h2>存储占用</h2><p>Local 与 S3 历史资产可混合统计。</p></div></header><div class="storage-compact"><article v-for="item in analytics?.storage" :key="item.driver" class="panel"><small>{{ item.driver.toUpperCase() }}</small><strong>{{ item.assetCount }} 张</strong><span>{{ fileSize(item.fileSizeBytes) }}</span></article><article v-if="!analytics?.storage.length" class="panel muted-cell">暂无图片资产</article></div></section>
      </div>
      <section class="analytics-section">
        <header class="log-header"><div><h2>Provider 请求日志</h2><p>仅保存结构化元数据，不保存 API Key、Prompt 或图片 Base64。</p></div><div class="log-filter"><n-input v-model:value="traceId" clearable placeholder="按 Trace ID 精确查询" @keyup.enter="filterLogs" /><n-button @click="filterLogs">查询</n-button></div></header>
        <div class="panel table-panel"><table><thead><tr><th>时间</th><th>路由</th><th>Provider / 模型</th><th>状态</th><th>耗时</th><th>Trace ID</th></tr></thead><tbody><tr v-for="item in logs" :key="item.id"><td>{{ dateTime(item.createdAt) }}</td><td><strong>{{ item.method }} {{ item.route }}</strong><small v-if="item.errorSummary" class="error-copy">{{ item.errorSummary }}</small></td><td><strong>{{ item.modelKey || '—' }}</strong><small>{{ item.providerType || '—' }}</small></td><td><span :class="item.statusCode && item.statusCode < 400 ? 'status-ok' : 'status-error'">{{ item.statusCode || '—' }}</span></td><td>{{ duration(item.latencyMs) }}</td><td><code>{{ item.traceId }}</code></td></tr><tr v-if="!logs.length"><td colspan="6" class="muted-cell">暂无请求日志</td></tr></tbody></table></div>
        <footer v-if="nextBeforeId" class="load-more"><n-button :loading="loadingMore" @click="loadMore">加载更多</n-button></footer>
      </section>
    </n-spin>
  </div>
</template>
