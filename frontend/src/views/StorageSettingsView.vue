<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { NButton, NInput, NSelect, NSwitch, useDialog, useMessage } from 'naive-ui'
import { api } from '@/api/client'

interface StorageView {
  activeDriver: 'local' | 's3'
  targetConfig: Record<string, unknown>
  localAssetCount: number
  s3AssetCount: number
  localPath: string
  s3Configured: boolean
}

interface ConsistencyRun {
  id: string
  status: 'running' | 'succeeded' | 'failed'
  deleteOrphans: boolean
  databaseAssets: number
  storageObjects: number
  missingObjects: number
  orphanObjects: number
  eligibleOrphans: number
  deletedOrphans: number
  errorMessage: string | null
  startedAt: string
  finishedAt: string | null
}

const view = ref<StorageView | null>(null)
const driver = ref<'local' | 's3'>('local')
const localPath = ref('./data/images')
const bucket = ref('')
const region = ref('auto')
const endpoint = ref('')
const prefix = ref('ai-image-studio/')
const forcePathStyle = ref(false)
const testing = ref(false)
const saving = ref(false)
const consistencyRuns = ref<ConsistencyRun[]>([])
const scanning = ref(false)
const message = useMessage()
const dialog = useDialog()

onMounted(async () => {
  await Promise.all([load(), loadConsistencyRuns()])
})

async function load() {
  const loaded = await api<StorageView>('/api/v1/admin/storage')
  view.value = loaded
  const target = loaded.targetConfig
  driver.value = String(target.driver ?? loaded.activeDriver) as 'local' | 's3'
  localPath.value = String(target.localPath ?? loaded.localPath)
  bucket.value = String(target.s3Bucket ?? '')
  region.value = String(target.s3Region ?? 'auto')
  endpoint.value = String(target.s3Endpoint ?? '')
  prefix.value = String(target.s3Prefix ?? 'ai-image-studio/')
  forcePathStyle.value = Boolean(target.s3ForcePathStyle)
}

function payload() {
  return { driver: driver.value, localPath: localPath.value, s3Bucket: bucket.value || null, s3Region: region.value, s3Endpoint: endpoint.value || null, s3Prefix: prefix.value, s3ForcePathStyle: forcePathStyle.value }
}

async function test() {
  testing.value = true
  try { await api('/api/v1/admin/storage/test', { method: 'POST', body: JSON.stringify(payload()) }); message.success('写入、读取、Head 和删除测试全部通过') }
  catch (error) { message.error(error instanceof Error ? error.message : '测试失败') }
  finally { testing.value = false }
}

function save() {
  dialog.warning({
    title: '保存目标存储配置', content: '配置将在应用重启后生效。切换到 S3 不会自动迁移现有 Local 图片，旧本地卷仍需保留。', positiveText: '确认保存', negativeText: '取消',
    onPositiveClick: async () => {
      saving.value = true
      try { await api('/api/v1/admin/storage', { method: 'PUT', body: JSON.stringify(payload()) }); message.success('配置已保存，重启应用后生效'); await load() }
      catch (error) { message.error(error instanceof Error ? error.message : '保存失败') }
      finally { saving.value = false }
    },
  })
}

async function loadConsistencyRuns() {
  consistencyRuns.value = await api<ConsistencyRun[]>('/api/v1/admin/storage/consistency')
}

async function runConsistencyScan(deleteOrphans: boolean) {
  scanning.value = true
  try {
    const run = await api<ConsistencyRun>('/api/v1/admin/storage/consistency/scan', {
      method: 'POST',
      body: JSON.stringify({ deleteOrphans }),
    })
    await loadConsistencyRuns()
    message.success(deleteOrphans ? `扫描完成，清理 ${run.deletedOrphans} 个过期孤儿文件` : '一致性扫描完成')
  } catch (error) {
    await loadConsistencyRuns().catch(() => undefined)
    message.error(error instanceof Error ? error.message : '一致性扫描失败')
  } finally {
    scanning.value = false
  }
}

function confirmOrphanCleanup() {
  dialog.warning({
    title: '扫描并清理孤儿文件',
    content: '只会删除超过安全宽限期、符合平台 Asset Key 格式且数据库无记录的文件。未知格式文件不会被删除。',
    positiveText: '确认扫描并清理',
    negativeText: '取消',
    onPositiveClick: () => runConsistencyScan(true),
  })
}
</script>

<template>
  <div class="page">
    <header class="page-header"><div><span class="eyebrow muted">ADMIN ONLY</span><h1>存储与系统设置</h1><p>设置新图片的目标存储；历史 Local/S3 数据可以同时读取。</p></div><span class="admin-badge">仅管理员可见</span></header>
    <div class="storage-stats">
      <article class="panel"><small>当前主驱动</small><strong>{{ view?.activeDriver?.toUpperCase() }}</strong><span>运行中配置</span></article>
      <article class="panel"><small>Local Asset</small><strong>{{ view?.localAssetCount ?? 0 }}</strong><span>切换后仍保留读取</span></article>
      <article class="panel"><small>S3 Asset</small><strong>{{ view?.s3AssetCount ?? 0 }}</strong><span>{{ view?.s3Configured ? '凭据已注入' : '凭据未配置' }}</span></article>
    </div>
    <section class="settings-card panel">
      <header><div><h2>目标存储</h2><p>Secret 只从环境变量或 Secret Manager 注入，页面不会回显。</p></div><n-select v-model:value="driver" class="driver-select" :options="[{label:'Local',value:'local'},{label:'S3 Compatible',value:'s3'}]" /></header>
      <div v-if="driver === 'local'" class="form-stack"><label>本地持久化目录<n-input v-model:value="localPath" /></label><p class="field-help">数据库只保存相对 storage_key，不保存此绝对路径。</p></div>
      <div v-else class="settings-grid">
        <label>Bucket<n-input v-model:value="bucket" /></label><label>Region<n-input v-model:value="region" /></label>
        <label class="wide">Endpoint<n-input v-model:value="endpoint" placeholder="AWS S3 可留空；兼容服务填写 HTTPS 地址" /></label>
        <label>Object Prefix<n-input v-model:value="prefix" /></label><label class="switch-label">Path Style<n-switch v-model:value="forcePathStyle" /></label>
        <div class="secret-state wide">S3 Access Key / Secret Key：{{ view?.s3Configured ? '已通过安全配置注入' : '尚未配置' }}</div>
      </div>
      <footer><n-button :loading="testing" @click="test">测试连接</n-button><n-button type="primary" :loading="saving" @click="save">保存配置</n-button></footer>
    </section>
    <section class="settings-card panel">
      <header>
        <div><h2>数据库 / 文件一致性</h2><p>检查数据库记录缺失文件和存储中的过期孤儿文件；自动任务默认每天运行一次。</p></div>
        <span v-if="consistencyRuns[0]" :class="['status', consistencyRuns[0].status]">{{ consistencyRuns[0].status === 'succeeded' ? '最近扫描成功' : consistencyRuns[0].status === 'failed' ? '最近扫描失败' : '扫描中' }}</span>
      </header>
      <div v-if="consistencyRuns[0]" class="storage-stats consistency-stats">
        <article><small>数据库 Asset</small><strong>{{ consistencyRuns[0].databaseAssets }}</strong></article>
        <article><small>实际文件</small><strong>{{ consistencyRuns[0].storageObjects }}</strong></article>
        <article><small>记录缺失文件</small><strong>{{ consistencyRuns[0].missingObjects }}</strong></article>
        <article><small>孤儿文件</small><strong>{{ consistencyRuns[0].orphanObjects }}</strong></article>
      </div>
      <p v-else class="field-help">尚未执行一致性扫描。</p>
      <div v-if="consistencyRuns.length" class="table-wrap">
        <table><thead><tr><th>时间</th><th>模式</th><th>缺失</th><th>孤儿</th><th>已清理</th><th>状态</th></tr></thead><tbody><tr v-for="run in consistencyRuns.slice(0, 5)" :key="run.id"><td>{{ new Date(run.startedAt).toLocaleString() }}</td><td>{{ run.deleteOrphans ? '扫描并清理' : '仅扫描' }}</td><td>{{ run.missingObjects }}</td><td>{{ run.orphanObjects }}</td><td>{{ run.deletedOrphans }}</td><td :title="run.errorMessage || undefined">{{ run.status }}</td></tr></tbody></table>
      </div>
      <footer><n-button :loading="scanning" @click="runConsistencyScan(false)">立即扫描</n-button><n-button type="warning" :loading="scanning" @click="confirmOrphanCleanup">扫描并清理孤儿文件</n-button></footer>
    </section>
  </div>
</template>
