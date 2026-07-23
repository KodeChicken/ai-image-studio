<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { NButton, NSpin, useMessage } from 'naive-ui'
import { api } from '@/api/client'

interface CurrencyTotal { currency: string; totalCost: number }
interface UsageOverview {
  period: { from: string; to: string }
  totals: { taskCount: number; imageCount: number }
  costs: CurrencyTotal[]
  byModel: Array<{ providerId: string; modelId: string; providerName: string; modelName: string; taskCount: number; imageCount: number; totalCost: number | null; currency: string }>
  recent: Array<{ id: number; taskId: string | null; providerName: string; modelName: string; quantity: number; unit: string; cost: number | null; currency: string; createdAt: string }>
  nextBeforeId: number | null
}

const overview = ref<UsageOverview | null>(null)
const loading = ref(false)
const loadingMore = ref(false)
const message = useMessage()
const costSummary = computed(() => overview.value?.costs.map(item => `${item.currency} ${item.totalCost.toFixed(4)}`).join(' · ') || '未配置模型价格')

onMounted(load)

async function load() {
  loading.value = true
  try {
    overview.value = await loadPage()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '用量加载失败')
  } finally {
    loading.value = false
  }
}

function loadPage(beforeId?: number) {
  const query = new URLSearchParams({ limit: '50' })
  if (beforeId) query.set('beforeId', String(beforeId))
  return api<UsageOverview>(`/api/v1/usage?${query}`)
}

async function loadMore() {
  if (!overview.value?.nextBeforeId) return
  loadingMore.value = true
  try {
    const page = await loadPage(overview.value.nextBeforeId)
    overview.value.recent.push(...page.recent)
    overview.value.nextBeforeId = page.nextBeforeId
  } catch (error) {
    message.error(error instanceof Error ? error.message : '更多用量记录加载失败')
  } finally {
    loadingMore.value = false
  }
}

function dateTime(value: string) {
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'medium' }).format(new Date(value))
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div><span class="eyebrow muted">USAGE & COST</span><h1>用量与成本</h1><p>统计最近 30 天的实际任务用量；成本仅按已配置的模型价格计算。</p></div>
      <n-button :loading="loading" @click="load">刷新</n-button>
    </header>
    <n-spin :show="loading">
      <div v-if="overview" class="metric-grid">
        <article class="panel metric-card"><small>完成任务</small><strong>{{ overview.totals.taskCount }}</strong><span>次</span></article>
        <article class="panel metric-card"><small>生成图片</small><strong>{{ overview.totals.imageCount }}</strong><span>张</span></article>
        <article class="panel metric-card wide-metric"><small>已计算成本</small><strong class="cost-copy">{{ costSummary }}</strong><span>不同币种分别汇总</span></article>
      </div>
      <section class="analytics-section">
        <header><div><h2>按模型汇总</h2><p>没有价格的模型仍记录图片数量，成本显示为“未配置”。</p></div></header>
        <div class="panel table-panel">
          <table>
            <thead><tr><th>Provider / 模型</th><th>任务</th><th>图片</th><th>成本</th></tr></thead>
            <tbody>
              <tr v-for="item in overview?.byModel" :key="`${item.providerId}-${item.modelId}-${item.currency}`">
                <td><strong>{{ item.modelName }}</strong><small>{{ item.providerName }}</small></td><td>{{ item.taskCount }}</td><td>{{ item.imageCount }}</td>
                <td>{{ item.totalCost == null ? '未配置' : `${item.currency} ${item.totalCost.toFixed(4)}` }}</td>
              </tr>
              <tr v-if="!overview?.byModel.length"><td colspan="4" class="muted-cell">当前周期暂无用量</td></tr>
            </tbody>
          </table>
        </div>
      </section>
      <section class="analytics-section">
        <header><div><h2>最近用量记录</h2><p>每条记录对应一次成功任务的实际图片输出。</p></div></header>
        <div class="panel table-panel">
          <table>
            <thead><tr><th>时间</th><th>模型</th><th>数量</th><th>成本</th><th>任务</th></tr></thead>
            <tbody>
              <tr v-for="item in overview?.recent" :key="item.id">
                <td>{{ dateTime(item.createdAt) }}</td><td><strong>{{ item.modelName }}</strong><small>{{ item.providerName }}</small></td><td>{{ item.quantity }} {{ item.unit }}</td>
                <td>{{ item.cost == null ? '未配置' : `${item.currency} ${item.cost.toFixed(4)}` }}</td><td><code>{{ item.taskId?.slice(0, 8) || '—' }}</code></td>
              </tr>
              <tr v-if="!overview?.recent.length"><td colspan="5" class="muted-cell">暂无记录</td></tr>
            </tbody>
          </table>
        </div>
        <footer v-if="overview?.nextBeforeId" class="load-more">
          <n-button :loading="loadingMore" @click="loadMore">加载更多</n-button>
        </footer>
      </section>
    </n-spin>
  </div>
</template>
