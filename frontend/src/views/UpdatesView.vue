<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { NAlert, NButton, NCheckbox, NModal, NProgress, NSpin, useMessage } from 'naive-ui'
import { api } from '@/api/client'

interface UpdateJob { id: string; action: 'upgrade' | 'rollback'; fromVersion: string | null; targetVersion: string; status: string; progress: number; currentStep: string | null; errorMessage: string | null; createdAt: string }
interface Deployment { id: string; appVersion: string; imageReference: string; imageDigest: string | null; schemaVersion: number; backupReference: string | null; deploymentStatus: string; deployedAt: string; rolledBackAt: string | null }
interface UpdateStatus { currentVersion: string; currentImage: string; schemaVersion: number; channel: string; manifestConfigured: boolean; updaterConfigured: boolean; keepPreviousReleases: number; jobs: UpdateJob[]; deployments: Deployment[] }
interface ReleaseManifest { version: string; image: string; imageDigest: string; schemaTarget: number; schemaMinSupported: number; schemaMaxSupported: number; rollbackCompatibleTo: string; requiresBackup: boolean; destructiveMigration: boolean; minimumUpdaterVersion: string; releaseNotes: string | null }
interface UpdateCheck { manifest: ReleaseManifest; hasUpdate: boolean; schemaCompatible: boolean }

const status = ref<UpdateStatus | null>(null)
const updateCheck = ref<UpdateCheck | null>(null)
const loading = ref(false)
const checking = ref(false)
const confirmOpen = ref(false)
const targetAction = ref<'upgrade' | 'rollback'>('upgrade')
const targetVersion = ref('')
const confirmDestructive = ref(false)
const submitting = ref(false)
const message = useMessage()
let pollTimer: number | undefined

const activeJob = computed(() => status.value?.jobs.find(item => item.status === 'pending' || item.status === 'running') || null)
const rollbackTargets = computed(() => status.value?.deployments.filter(item => item.appVersion !== status.value?.currentVersion && ['active', 'superseded'].includes(item.deploymentStatus)).slice(0, status.value?.keepPreviousReleases || 3) || [])

onMounted(load)
onBeforeUnmount(() => window.clearTimeout(pollTimer))

async function load() {
  loading.value = true
  try {
    status.value = await api('/api/v1/admin/updates/status')
    schedulePoll()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '升级状态加载失败')
  } finally {
    loading.value = false
  }
}

async function checkUpdate() {
  checking.value = true
  try {
    const result = await api<UpdateCheck>('/api/v1/admin/updates/check', { method: 'POST' })
    updateCheck.value = result
    message.success(result.hasUpdate ? `发现 ${result.manifest.version}` : '当前已是最新版本')
  } catch (error) {
    message.error(error instanceof Error ? error.message : '检查更新失败')
  } finally {
    checking.value = false
  }
}

function openAction(action: 'upgrade' | 'rollback', version: string) {
  targetAction.value = action
  targetVersion.value = version
  confirmDestructive.value = false
  confirmOpen.value = true
}

async function submitAction() {
  submitting.value = true
  try {
    await api('/api/v1/admin/updates/jobs', {
      method: 'POST',
      headers: { 'X-AI-Studio-Action': 'update' },
      body: JSON.stringify({ action: targetAction.value, targetVersion: targetVersion.value, confirmDestructiveMigration: confirmDestructive.value }),
    })
    confirmOpen.value = false
    message.success('Host Updater 已接受任务')
    await load()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '提交升级任务失败')
  } finally {
    submitting.value = false
  }
}

function schedulePoll() {
  window.clearTimeout(pollTimer)
  if (!activeJob.value) return
  pollTimer = window.setTimeout(async () => {
    try {
      await api(`/api/v1/admin/updates/jobs/${activeJob.value?.id}`)
      status.value = await api('/api/v1/admin/updates/status')
    } finally {
      schedulePoll()
    }
  }, 2500)
}

function dateTime(value: string) { return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'medium' }).format(new Date(value)) }
</script>

<template>
  <div class="page">
    <header class="page-header"><div><span class="eyebrow muted">ADMIN UPDATE CONTROL</span><h1>版本与升级</h1><p>应用仅负责检查、审计与发起请求；备份、镜像切换和回滚由独立 Host Updater 执行。</p></div><span class="admin-badge">仅管理员可见</span></header>
    <n-spin :show="loading">
      <n-alert v-if="status && !status.updaterConfigured" type="warning" title="Host Updater 未配置" class="page-alert">当前只可检查版本，不能升级或回滚。Web 容器不会直接访问 Docker Socket。</n-alert>
      <div v-if="status" class="storage-stats update-stats">
        <article class="panel"><small>当前版本</small><strong>{{ status.currentVersion }}</strong><span>{{ status.currentImage }}</span></article>
        <article class="panel"><small>数据库 Schema</small><strong>{{ status.schemaVersion }}</strong><span>SQLx Migration</span></article>
        <article class="panel"><small>更新通道</small><strong>{{ status.channel }}</strong><span>保留前 {{ status.keepPreviousReleases }} 个版本</span></article>
      </div>
      <section class="panel settings-card update-card">
        <header><div><h2>Release Manifest</h2><p>版本信息仅用于展示与兼容性预检；Host Updater 会再次验证签名、Digest、备份和健康状态。</p></div><n-button :disabled="!status?.manifestConfigured" :loading="checking" @click="checkUpdate">检查更新</n-button></header>
        <div v-if="updateCheck" class="release-summary">
          <div><small>最新版本</small><strong>{{ updateCheck.manifest.version }}</strong></div><div><small>目标 Schema</small><strong>{{ updateCheck.manifest.schemaTarget }}</strong></div><div><small>备份要求</small><strong>{{ updateCheck.manifest.requiresBackup ? '必须' : '按策略' }}</strong></div><div><small>Migration</small><strong :class="{ 'status-error': updateCheck.manifest.destructiveMigration }">{{ updateCheck.manifest.destructiveMigration ? '包含破坏性变更' : '兼容性变更' }}</strong></div>
          <p v-if="updateCheck.manifest.releaseNotes">{{ updateCheck.manifest.releaseNotes }}</p>
          <n-alert v-if="!updateCheck.schemaCompatible" type="error">当前 Schema 不满足目标版本最低要求。</n-alert>
          <n-button type="primary" :disabled="!updateCheck.hasUpdate || !updateCheck.schemaCompatible || !status?.updaterConfigured || Boolean(activeJob)" @click="openAction('upgrade', updateCheck.manifest.version)">升级到 {{ updateCheck.manifest.version }}</n-button>
        </div>
        <p v-else class="field-help">{{ status?.manifestConfigured ? '点击“检查更新”读取 Release Manifest。' : '请在服务端配置 UPDATE_MANIFEST_URL。' }}</p>
      </section>
      <section v-if="activeJob" class="panel settings-card update-card active-update"><header><div><h2>{{ activeJob.action === 'upgrade' ? '升级' : '回滚' }}进行中</h2><p>{{ activeJob.currentStep || '等待 Host Updater' }}</p></div><strong>{{ activeJob.targetVersion }}</strong></header><n-progress type="line" :percentage="activeJob.progress" :status="activeJob.status === 'failed' ? 'error' : 'default'" /><p v-if="activeJob.errorMessage" class="status-error">{{ activeJob.errorMessage }}</p></section>
      <section class="analytics-section"><header><div><h2>可回滚版本</h2><p>只展示 Host Updater 已保留并写入部署历史的最近版本。</p></div></header><div class="panel table-panel"><table><thead><tr><th>版本</th><th>Schema</th><th>镜像</th><th>部署时间</th><th></th></tr></thead><tbody><tr v-for="item in rollbackTargets" :key="item.id"><td><strong>{{ item.appVersion }}</strong><small>{{ item.deploymentStatus }}</small></td><td>{{ item.schemaVersion }}</td><td><code>{{ item.imageDigest || item.imageReference }}</code></td><td>{{ dateTime(item.deployedAt) }}</td><td><n-button size="small" :disabled="!status?.updaterConfigured || Boolean(activeJob)" @click="openAction('rollback', item.appVersion)">回滚</n-button></td></tr><tr v-if="!rollbackTargets.length"><td colspan="5" class="muted-cell">暂无可回滚部署记录</td></tr></tbody></table></div></section>
      <section class="analytics-section"><header><div><h2>升级审计</h2><p>保留最近 20 次升级或回滚请求。</p></div></header><div class="panel table-panel"><table><thead><tr><th>时间</th><th>动作</th><th>目标版本</th><th>状态</th><th>进度</th><th>步骤/错误</th></tr></thead><tbody><tr v-for="item in status?.jobs" :key="item.id"><td>{{ dateTime(item.createdAt) }}</td><td>{{ item.action }}</td><td>{{ item.targetVersion }}</td><td>{{ item.status }}</td><td>{{ item.progress }}%</td><td><span :class="{ 'status-error': item.errorMessage }">{{ item.errorMessage || item.currentStep || '—' }}</span></td></tr><tr v-if="!status?.jobs.length"><td colspan="6" class="muted-cell">暂无升级任务</td></tr></tbody></table></div></section>
    </n-spin>
  </div>
  <n-modal v-model:show="confirmOpen" preset="card" :title="targetAction === 'upgrade' ? '确认升级' : '确认回滚'" class="dialog-card"><div class="form-stack"><n-alert type="warning">目标版本：{{ targetVersion }}。Host Updater 将独立执行备份、迁移、健康检查和失败恢复。</n-alert><n-checkbox v-if="targetAction === 'upgrade' && updateCheck?.manifest.destructiveMigration" v-model:checked="confirmDestructive">我已确认该版本包含破坏性 Migration</n-checkbox><n-button type="primary" :loading="submitting" @click="submitAction">提交给 Host Updater</n-button></div></n-modal>
</template>
