<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { NButton, NInput, NInputNumber, NModal, NSelect, NSwitch, useMessage } from 'naive-ui'
import { api, streamPost, streamTask } from '@/api/client'
import ImageCropModal, { type CropPreviewImage } from '@/components/ImageCropModal.vue'
import ImageSizeControl from '@/components/ImageSizeControl.vue'
import {
  branchPath,
  branchPosition,
  latestDescendantId,
  latestMessageId,
} from '@/lib/conversationBranches'
import { parseImageSize } from '@/lib/imageSizing'
import type {
  Conversation,
  ConversationDetail,
  ConversationMessage,
  ImageAsset,
  ImageModel,
  ParameterDefinition,
  PromptTemplate,
  Provider,
  TaskEvent,
} from '@/types/api'

const message = useMessage()
const conversations = ref<Conversation[]>([])
const activeConversation = ref<ConversationDetail | null>(null)
const providers = ref<Provider[]>([])
const models = ref<ImageModel[]>([])
const templates = ref<PromptTemplate[]>([])
const conversationSearch = ref('')
const providerId = ref<string | null>(null)
const modelId = ref<string | null>(null)
const prompt = ref('')
const styleId = ref<string | null>(null)
interface ComposerAttachment {
  file: File
  previewUrl: string
}
const files = ref<ComposerAttachment[]>([])
const fileInput = ref<HTMLInputElement | null>(null)
interface PendingUserMessage {
  content: string
  createdAt: string
}
interface ConversationTaskState {
  sending: boolean
  activeTaskId: string | null
  cancelling: boolean
  retryingTaskId: string | null
  taskStage: string
  taskElapsedSeconds: number
  pendingUserMessage: PendingUserMessage | null
  partialPreview: { contentUrl: string; label: string } | null
}
const conversationTaskStates = reactive<Record<string, ConversationTaskState>>({})
const taskTimers = new Map<string, ReturnType<typeof setInterval>>()
const messageTimeFormatter = new Intl.DateTimeFormat('zh-CN', {
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hour12: false,
})
const editingTitle = ref<string | null>(null)
const titleDraft = ref('')
const draggedIndex = ref<number | null>(null)
const timeline = ref<HTMLElement | null>(null)
const composerInput = ref<HTMLTextAreaElement | null>(null)
const activeLeafId = ref<string | null>(null)
const composerParentId = ref<string | null>(null)
const templateManagerOpen = ref(false)
const editingTemplateId = ref<string | null>(null)
const templateTitle = ref('')
const templatePrompt = ref('')
const imagePreviewOpen = ref(false)
const imagePreview = ref<CropPreviewImage | null>(null)
const imagePreviewMode = ref<'preview' | 'crop'>('preview')
const mobilePanel = ref<'conversations' | 'parameters' | null>(null)
const parameterPanelWidth = ref(340)
const viewportWidth = ref(window.innerWidth)
const resizingParameterPanel = ref(false)
type ParameterValue = string | number | boolean | null
type ParameterMemory = Record<string, Record<string, ParameterValue>>
const parameterMemoryKey = 'studio-generation-parameters-v1'
const parameters = reactive<Record<string, ParameterValue>>({ aspect_ratio: 'auto' })
const supportedImageTypes = new Set(['image/png', 'image/jpeg', 'image/webp'])
const parameterPanelMinWidth = 320
const parameterPanelMaxLimit = 760
let stopParameterResize: (() => void) | null = null
let conversationSelectionVersion = 0

const conversationMessages = computed(() => activeConversation.value?.messages ?? [])
const activeTaskState = computed(() => (
  activeConversation.value ? conversationTaskStates[activeConversation.value.id] ?? null : null
))
const sending = computed(() => activeTaskState.value?.sending ?? false)
const activeTaskId = computed(() => activeTaskState.value?.activeTaskId ?? null)
const cancelling = computed(() => activeTaskState.value?.cancelling ?? false)
const retryingTaskId = computed(() => activeTaskState.value?.retryingTaskId ?? null)
const taskStage = computed(() => activeTaskState.value?.taskStage ?? '')
const taskElapsedSeconds = computed(() => activeTaskState.value?.taskElapsedSeconds ?? 0)
const partialPreview = computed(() => activeTaskState.value?.partialPreview ?? null)
const filteredConversations = computed(() => {
  const query = conversationSearch.value.trim().toLocaleLowerCase()
  return query
    ? conversations.value.filter((item) => item.title.toLocaleLowerCase().includes(query))
    : conversations.value
})
const visibleMessages = computed(() => branchPath(conversationMessages.value, activeLeafId.value))
const visiblePendingUserMessage = computed(() => activeTaskState.value?.pendingUserMessage ?? null)
const visibleLeafId = computed(() => visibleMessages.value[visibleMessages.value.length - 1]?.id ?? null)
const currentBranchParentId = computed(() => {
  if (composerParentId.value) return composerParentId.value
  return [...visibleMessages.value].reverse().find((item) => item.role === 'assistant')?.id ?? null
})
const composerAnchor = computed(() =>
  composerParentId.value
    ? conversationMessages.value.find((item) => item.id === composerParentId.value) ?? null
    : null,
)
const currentModel = computed(() => models.value.find((item) => item.id === modelId.value) ?? null)
const currentTemplate = computed(() => templates.value.find((item) => item.id === styleId.value) ?? null)
const providerOptions = computed(() => providers.value.map((item) => ({ label: item.displayName, value: item.id })))
const modelOptions = computed(() =>
  models.value
    .filter((item) => item.providerId === providerId.value && item.enabled)
    .map((item) => ({
      label: `${item.displayName}${item.availabilityStatus === 'verified' ? '' : ' · 待分类'}`,
      value: item.id,
      disabled: item.availabilityStatus !== 'verified',
    })),
)
const styleOptions = computed(() => [
  { label: '不使用风格模板', value: '' },
  ...templates.value.map((item) => ({ label: item.title, value: item.id })),
])
const schema = computed(() => currentModel.value?.parameterSchema.parameters ?? {})
const advancedParameters = computed(() =>
  Object.entries(schema.value).filter(
    ([name, definition]) =>
      !['aspect_ratio', 'size', 'quality', 'n'].includes(name) && isParameterVisible(name, definition),
  ),
)
const qualityOptions = computed(() => enumOptions('quality', ['auto', 'low', 'medium', 'high']))
const taskStatusWithElapsed = computed(
  () => `${taskStage.value || '模型正在生成'} · ${formatDuration(taskElapsedSeconds.value)}`,
)
const parameterPanelMaxWidth = computed(() =>
  Math.max(parameterPanelMinWidth, Math.min(parameterPanelMaxLimit, viewportWidth.value - 72 - 286 - 420)),
)

watch(providerId, async (value) => {
  const available = models.value.filter((item) => item.providerId === value && item.availabilityStatus === 'verified')
  if (!available.some((item) => item.id === modelId.value)) modelId.value = available[0]?.id ?? null
})

watch(modelId, (value) => {
  for (const key of Object.keys(parameters)) delete parameters[key]
  parameters.aspect_ratio = 'auto'
  for (const [key, definition] of Object.entries(schema.value)) {
    const value = definition.default
    if (
      typeof value === 'string' ||
      typeof value === 'number' ||
      typeof value === 'boolean' ||
      value === null
    ) {
      parameters[key] = value
    }
  }
  if (!value) return
  const remembered = readParameterMemory()[value]
  if (!remembered) return
  for (const [key, rememberedValue] of Object.entries(remembered)) {
    const definition = schema.value[key]
    if (definition && acceptsParameterValue(definition, rememberedValue)) {
      parameters[key] = rememberedValue
    }
  }
})

watch(parameterPanelWidth, (value) => {
  document.documentElement.style.setProperty('--studio-parameter-panel-width', `${value}px`)
}, { immediate: true })

onMounted(async () => {
  window.addEventListener('resize', updateParameterPanelBounds)
  window.addEventListener('keydown', closeMobilePanelOnEscape)
  updateParameterPanelBounds()
  await Promise.all([loadConversations(), loadProviders(), loadModels(), loadTemplates()])
  if (!providerId.value) providerId.value = providers.value[0]?.id ?? null
  if (conversations.value[0]) await selectConversation(conversations.value[0].id)
})

onBeforeUnmount(() => {
  for (const conversationId of taskTimers.keys()) stopTaskTimer(conversationId)
  stopParameterResize?.()
  files.value.forEach((attachment) => URL.revokeObjectURL(attachment.previewUrl))
  window.removeEventListener('resize', updateParameterPanelBounds)
  window.removeEventListener('keydown', closeMobilePanelOnEscape)
  document.documentElement.style.removeProperty('--studio-parameter-panel-width')
})

function taskStateFor(conversationId: string) {
  if (!conversationTaskStates[conversationId]) {
    conversationTaskStates[conversationId] = {
      sending: false,
      activeTaskId: null,
      cancelling: false,
      retryingTaskId: null,
      taskStage: '',
      taskElapsedSeconds: 0,
      pendingUserMessage: null,
      partialPreview: null,
    }
  }
  return conversationTaskStates[conversationId]
}

function startTaskTimer(conversationId: string, state: ConversationTaskState, startedAt = Date.now()) {
  stopTaskTimer(conversationId)
  const update = () => {
    state.taskElapsedSeconds = Math.max(0, Math.floor((Date.now() - startedAt) / 1000))
  }
  update()
  taskTimers.set(conversationId, setInterval(update, 1000))
}

function stopTaskTimer(conversationId: string) {
  const timer = taskTimers.get(conversationId)
  if (timer) clearInterval(timer)
  taskTimers.delete(conversationId)
}

function formatDuration(seconds: number) {
  const total = Math.max(0, Math.floor(seconds))
  if (total < 60) return `${total}秒`
  const minutes = Math.floor(total / 60)
  const remaining = total % 60
  return remaining ? `${minutes}分${remaining}秒` : `${minutes}分`
}

function formatMessageTime(value: string) {
  return messageTimeFormatter.format(new Date(value))
}

function messageTimeText(item: ConversationMessage) {
  const timestamp = item.role === 'assistant' && item.taskFinishedAt
    ? item.taskFinishedAt
    : item.createdAt
  const prefix = item.role === 'user' ? '发送' : item.taskFinishedAt ? '完成' : '创建'
  if (!item.taskStartedAt || !item.taskFinishedAt) return `${prefix} ${formatMessageTime(timestamp)}`
  const duration = Math.max(
    0,
    Math.round((new Date(item.taskFinishedAt).getTime() - new Date(item.taskStartedAt).getTime()) / 1000),
  )
  return `${prefix} ${formatMessageTime(timestamp)} · 耗时 ${formatDuration(duration)}`
}

async function loadConversations() {
  conversations.value = await api<Conversation[]>('/api/v1/conversations')
}

async function loadProviders() {
  providers.value = await api<Provider[]>('/api/v1/providers')
}

async function loadModels() {
  models.value = await api<ImageModel[]>('/api/v1/models?includeDiscovered=true')
}

async function loadTemplates() {
  templates.value = await api<PromptTemplate[]>('/api/v1/prompt-templates?templateType=style')
}

async function selectConversation(id: string) {
  const selectionVersion = ++conversationSelectionVersion
  const selected = await api<ConversationDetail>(`/api/v1/conversations/${id}`)
  if (selectionVersion !== conversationSelectionVersion) return
  applySelectedConversation(selected)
  await scrollBottom()
}

function applySelectedConversation(selected: ConversationDetail) {
  activeConversation.value = selected
  activeLeafId.value = latestMessageId(selected.messages)
  composerParentId.value = null
  providerId.value = selected.defaultProviderId ?? providerId.value
  modelId.value = selected.defaultModelId ?? modelId.value
  mobilePanel.value = null
}

async function refreshConversationIfActive(conversationId: string) {
  const refreshed = await api<ConversationDetail>(`/api/v1/conversations/${conversationId}`)
  if (activeConversation.value?.id === conversationId) {
    applySelectedConversation(refreshed)
    await scrollBottom()
  }
  return refreshed
}

async function createConversation() {
  const created = await api<Conversation>('/api/v1/conversations', {
    method: 'POST',
    body: JSON.stringify({
      title: '新会话',
      defaultProviderId: providerId.value,
      defaultModelId: modelId.value,
    }),
  })
  conversations.value.push(created)
  await selectConversation(created.id)
}

function beginTitle(item: Conversation) {
  editingTitle.value = item.id
  titleDraft.value = item.title
}

async function saveTitle(item: Conversation) {
  const title = titleDraft.value.trim()
  if (!title) return
  const updated = await api<Conversation>(`/api/v1/conversations/${item.id}`, {
    method: 'PATCH',
    body: JSON.stringify({ title }),
  })
  Object.assign(item, updated)
  if (activeConversation.value?.id === item.id) activeConversation.value.title = title
  editingTitle.value = null
}

async function dropConversation(targetIndex: number) {
  if (draggedIndex.value === null || draggedIndex.value === targetIndex) return
  const [moved] = conversations.value.splice(draggedIndex.value, 1)
  if (!moved) return
  conversations.value.splice(targetIndex, 0, moved)
  draggedIndex.value = null
  await api<void>('/api/v1/conversations/order', {
    method: 'PUT',
    body: JSON.stringify({ conversationIds: conversations.value.map((item) => item.id) }),
  })
}

function appendFiles(selectedFiles: File[]) {
  const supportedFiles = selectedFiles.filter((file) => supportedImageTypes.has(file.type))
  if (supportedFiles.length !== selectedFiles.length) message.warning('仅支持 PNG、JPEG 和 WebP 图片')
  files.value.push(...supportedFiles.map((file) => ({ file, previewUrl: URL.createObjectURL(file) })))
  return supportedFiles.length
}

function chooseFiles(event: Event) {
  const input = event.target as HTMLInputElement
  appendFiles(Array.from(input.files ?? []))
  input.value = ''
}

function pasteFiles(event: ClipboardEvent) {
  const pastedImages = Array.from(event.clipboardData?.files ?? []).filter((file) => file.type.startsWith('image/'))
  if (!pastedImages.length) return
  event.preventDefault()
  const addedCount = appendFiles(pastedImages)
  if (addedCount) message.success(`已粘贴 ${addedCount} 张参考图`)
}

function removeFile(index: number) {
  const [removed] = files.value.splice(index, 1)
  if (removed) URL.revokeObjectURL(removed.previewUrl)
}

function openImagePreview(asset: ImageAsset, label: string, mode: 'preview' | 'crop' = 'preview') {
  imagePreview.value = {
    id: asset.id,
    contentUrl: asset.contentUrl,
    label,
    metadata: `${asset.width} × ${asset.height} · ${asset.mimeType}`,
    mimeType: asset.mimeType,
    width: asset.width,
    height: asset.height,
  }
  imagePreviewMode.value = mode
  imagePreviewOpen.value = true
}

function openPartialPreview(contentUrl: string, label: string) {
  imagePreview.value = { contentUrl, label, metadata: '最终原图仍在生成' }
  imagePreviewMode.value = 'preview'
  imagePreviewOpen.value = true
}

function imageDownloadName(id: string, mimeType: string) {
  const extension = mimeType === 'image/jpeg' ? 'jpg' : mimeType === 'image/webp' ? 'webp' : 'png'
  return `ai-image-studio-${id}.${extension}`
}

function visibleMessageAssets(item: ConversationMessage) {
  const relationType = item.role === 'user'
    ? 'attachment'
    : item.role === 'assistant'
      ? 'generated'
      : null
  return relationType ? item.assets.filter((asset) => asset.relationType === relationType) : []
}

function messageImageLabel(item: ConversationMessage) {
  return item.role === 'user' ? '参考图' : '生成图片'
}

function updateParameterPanelBounds() {
  viewportWidth.value = window.innerWidth
  parameterPanelWidth.value = Math.min(parameterPanelWidth.value, parameterPanelMaxWidth.value)
  if (
    (mobilePanel.value === 'conversations' && viewportWidth.value > 860)
    || (mobilePanel.value === 'parameters' && viewportWidth.value > 1220)
  ) mobilePanel.value = null
}

function closeMobilePanelOnEscape(event: KeyboardEvent) {
  if (event.key === 'Escape') mobilePanel.value = null
}

function resizeParameterPanel(delta: number) {
  parameterPanelWidth.value = Math.min(
    parameterPanelMaxWidth.value,
    Math.max(parameterPanelMinWidth, parameterPanelWidth.value + delta),
  )
}

function startParameterResize(event: PointerEvent) {
  if (viewportWidth.value <= 1220) return
  event.preventDefault()
  stopParameterResize?.()
  const startX = event.clientX
  const startWidth = parameterPanelWidth.value
  const move = (moveEvent: PointerEvent) => {
    parameterPanelWidth.value = Math.min(
      parameterPanelMaxWidth.value,
      Math.max(parameterPanelMinWidth, Math.round(startWidth + startX - moveEvent.clientX)),
    )
  }
  const finish = () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    document.body.classList.remove('parameter-panel-resizing')
    resizingParameterPanel.value = false
    stopParameterResize = null
  }
  resizingParameterPanel.value = true
  document.body.classList.add('parameter-panel-resizing')
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', finish)
  stopParameterResize = finish
}

async function send() {
  const content = prompt.value.trim()
  if (!content || sending.value) return
  await submitMessage({
    content,
    parentMessageId: currentBranchParentId.value,
    inputAssetIds: [],
    useComposerFiles: true,
  })
}

interface MessageSubmission {
  content: string
  parentMessageId: string | null
  inputAssetIds: string[]
  useComposerFiles: boolean
}

async function submitMessage(submission: MessageSubmission) {
  const selectedProviderId = providerId.value
  const selectedModelId = modelId.value
  if (!selectedProviderId || !selectedModelId) return message.error('请先配置并选择可用模型')
  if (!activeConversation.value) await createConversation()
  const conversationId = activeConversation.value!.id
  const state = taskStateFor(conversationId)
  if (state.sending) return
  const knownMessageIds = new Set(conversationMessages.value.map((item) => item.id))
  const composerFiles = submission.useComposerFiles ? [...files.value] : []
  const requestParameters = cleanParameters(submission.inputAssetIds.length > 0 || composerFiles.length > 0)
  const stylePrompt = currentTemplate.value?.prompt
  const uploadedAssetIds: string[] = []
  state.sending = true
  startTaskTimer(conversationId, state)
  state.activeTaskId = null
  state.cancelling = false
  state.retryingTaskId = null
  state.taskStage = '正在创建任务'
  state.partialPreview = null
  state.pendingUserMessage = {
    content: submission.content,
    createdAt: new Date().toISOString(),
  }
  if (submission.useComposerFiles) {
    prompt.value = ''
    files.value = []
    if (fileInput.value) fileInput.value.value = ''
  }
  try {
    await scrollBottom()
    const inputAssetIds = [...submission.inputAssetIds]
    if (submission.useComposerFiles) {
      for (const attachment of composerFiles) {
        const body = new FormData()
        body.append('file', attachment.file)
        const asset = await api<ImageAsset>('/api/v1/image-assets/uploads', { method: 'POST', body })
        inputAssetIds.push(asset.id)
        uploadedAssetIds.push(asset.id)
      }
    }
    await streamPost(
      `/api/v1/conversations/${conversationId}/messages`,
      {
        content: submission.content,
        parentMessageId: submission.parentMessageId,
        providerId: selectedProviderId,
        modelId: selectedModelId,
        parameters: requestParameters,
        inputAssetIds,
        stylePrompt,
        stream: true,
      },
      (event) => handleTaskEvent(conversationId, state, event),
    )
    state.pendingUserMessage = null
    await refreshConversationIfActive(conversationId)
    await loadConversations()
  } catch (error) {
    await Promise.allSettled(
      uploadedAssetIds.map((id) => api<void>(`/api/v1/image-assets/${id}`, { method: 'DELETE' })),
    )
    let submissionPersisted = Boolean(state.activeTaskId)
    try {
      const refreshed = await refreshConversationIfActive(conversationId)
      submissionPersisted ||= refreshed.messages.some(
        (item) =>
          !knownMessageIds.has(item.id) &&
          item.role === 'user' &&
          item.content?.trim() === submission.content,
      )
      await loadConversations()
    } catch {
      // Keep the original request locally when the server state cannot be refreshed.
    }
    if (submission.useComposerFiles && !submissionPersisted && activeConversation.value?.id === conversationId) {
      if (!prompt.value.trim()) prompt.value = submission.content
      if (!files.value.length) files.value = composerFiles
    }
    message.error(error instanceof Error ? error.message : '生成失败')
  } finally {
    const activePreviewUrls = new Set(files.value.map((attachment) => attachment.previewUrl))
    composerFiles.forEach((attachment) => {
      if (!activePreviewUrls.has(attachment.previewUrl)) URL.revokeObjectURL(attachment.previewUrl)
    })
    stopTaskTimer(conversationId)
    state.sending = false
    state.activeTaskId = null
    state.cancelling = false
    state.partialPreview = null
    state.pendingUserMessage = null
    const completedStage = state.taskStage
    setTimeout(() => {
      if (!state.sending && state.taskStage === completedStage) state.taskStage = ''
    }, 1800)
  }
}

function handleTaskEvent(conversationId: string, state: ConversationTaskState, event: TaskEvent) {
  if (event.type === 'stream.reconnecting') state.taskStage = '正在恢复连接'
  if (event.type === 'stream.polling') state.taskStage = '正在确认任务状态'
  if (event.type === 'task.created') {
    state.taskStage = '等待生成资源'
    if (typeof event.data.taskId === 'string') state.activeTaskId = event.data.taskId
  }
  if (event.type === 'task.progress') {
    const stage = String(event.data.stage ?? '')
    if (stage === 'provider.processing') startTaskTimer(conversationId, state)
    state.taskStage = stageText(stage)
  }
  if (event.type === 'image.partial' && typeof event.data.contentUrl === 'string') {
    state.partialPreview = {
      contentUrl: event.data.contentUrl,
      label: `流式局部预览 ${Number(event.data.partialIndex ?? 0) + 1}`,
    }
    state.taskStage = '模型正在细化图片'
    if (activeConversation.value?.id === conversationId) void scrollBottom()
  }
  if (event.type === 'image.completed') {
    state.taskStage = '正在保存原图'
    const asset = event.data.asset
    if (asset && typeof asset === 'object' && 'contentUrl' in asset && typeof asset.contentUrl === 'string') {
      state.partialPreview = { contentUrl: asset.contentUrl, label: '最终图片' }
    }
  }
  if (event.type === 'task.completed') state.taskStage = '已完成'
  if (event.type === 'task.failed') state.taskStage = '生成失败'
  if (event.type === 'task.cancelled') {
    state.taskStage = '已取消'
    state.partialPreview = null
  }
}

async function cancelActiveTask() {
  const state = activeTaskState.value
  if (!state?.activeTaskId || state.cancelling) return
  state.cancelling = true
  try {
    await api<void>(`/api/v1/tasks/${state.activeTaskId}/cancel`, { method: 'POST' })
    state.taskStage = '正在取消'
    state.partialPreview = null
  } catch (error) {
    state.cancelling = false
    message.error(error instanceof Error ? error.message : '取消任务失败')
  }
}

async function retryTask(item: ConversationMessage) {
  const conversationId = activeConversation.value?.id
  if (!conversationId || !item.taskId) return
  const state = taskStateFor(conversationId)
  if (state.sending) return
  state.sending = true
  startTaskTimer(conversationId, state)
  state.retryingTaskId = item.taskId
  state.activeTaskId = item.taskId
  state.cancelling = false
  state.taskStage = '正在重新生成'
  state.partialPreview = null
  try {
    const result = await api<{ taskId: string; lastEventId: number }>(
      `/api/v1/tasks/${item.taskId}/retry`,
      { method: 'POST' },
    )
    item.status = 'streaming'
    item.content = null
    item.taskErrorCode = null
    item.taskErrorMessage = null
    item.taskRetryCount = (item.taskRetryCount ?? 0) + 1
    item.taskStartedAt = null
    item.taskFinishedAt = null
    state.activeTaskId = result.taskId
    await streamTask(result.taskId, (event) => handleTaskEvent(conversationId, state, event), {
      initialLastEventId: String(result.lastEventId),
    })
    await refreshConversationIfActive(conversationId)
    await loadConversations()
  } catch (error) {
    message.error(error instanceof Error ? error.message : '重试任务失败')
    await refreshConversationIfActive(conversationId).catch(() => undefined)
  } finally {
    stopTaskTimer(conversationId)
    state.sending = false
    state.retryingTaskId = null
    state.activeTaskId = null
    state.cancelling = false
    state.partialPreview = null
    const completedStage = state.taskStage
    setTimeout(() => {
      if (!state.sending && state.taskStage === completedStage) state.taskStage = ''
    }, 1800)
  }
}

function messageBranch(item: ConversationMessage) {
  return branchPosition(conversationMessages.value, item)
}

async function selectSiblingBranch(item: ConversationMessage, offset: number) {
  const branch = messageBranch(item)
  const sibling = branch.siblings[branch.index + offset]
  if (!sibling) return
  activeLeafId.value = latestDescendantId(conversationMessages.value, sibling.id)
  composerParentId.value = null
  await scrollBottom()
}

async function continueFrom(item: ConversationMessage) {
  if (item.role !== 'assistant') return
  activeLeafId.value = item.id
  composerParentId.value = item.id
  await nextTick()
  composerInput.value?.focus()
}

async function resetComposerBranch() {
  activeLeafId.value = latestMessageId(conversationMessages.value)
  composerParentId.value = null
  await scrollBottom()
}

function canRegenerate(item: ConversationMessage) {
  if (item.role !== 'assistant' || item.status === 'streaming') return false
  const request = conversationMessages.value.find((candidate) => candidate.id === item.parentMessageId)
  return request?.role === 'user' && Boolean(request.content?.trim())
}

async function regenerate(item: ConversationMessage) {
  if (sending.value) return
  const request = conversationMessages.value.find((candidate) => candidate.id === item.parentMessageId)
  const content = request?.content?.trim()
  if (request?.role !== 'user' || !content) return message.error('找不到这次生成对应的用户消息')
  composerParentId.value = null
  await submitMessage({
    content,
    parentMessageId: request.parentMessageId,
    inputAssetIds: request.assets.map((asset) => asset.id),
    useComposerFiles: false,
  })
}

function cleanParameters(hasInputImages = files.value.length > 0) {
  const exactSize = parseImageSize(parameters.size)
  return Object.fromEntries(
    Object.entries(parameters).filter(([name, value]) => {
      const definition = schema.value[name]
      return (
        definition !== undefined &&
        !(name === 'aspect_ratio' && exactSize) &&
        isParameterVisible(name, definition, hasInputImages) &&
        value !== '' &&
        value !== null &&
        value !== undefined
      )
    }),
  )
}

function isParameterVisible(
  _name: string,
  definition: ParameterDefinition,
  hasInputImages = files.value.length > 0,
) {
  if (definition.supported === false) return false
  const operation = hasInputImages ? 'edit' : 'generation'
  if (definition.operations && !definition.operations.includes(operation)) return false
  return Object.entries(definition.visible_when ?? {}).every(([dependency, expected]) => {
    const actual = dependency === 'stream' ? true : parameters[dependency] ?? schema.value[dependency]?.default
    return Array.isArray(expected) ? expected.includes(actual) : actual === expected
  })
}

function enumOptions(name: string, fallback: string[]) {
  const options = schema.value[name]?.options ?? fallback
  return options.map((value) => ({ label: value === 'auto' ? 'Auto（默认）' : value, value }))
}

function parameterOptions(definition: ParameterDefinition) {
  return (definition.options ?? []).map((value) => ({
    label: value === 'auto' ? 'Auto（默认）' : value,
    value,
  }))
}

function selectValue(value: ParameterValue | undefined): string | number | null {
  return typeof value === 'string' || typeof value === 'number' ? value : null
}

function numberValue(value: ParameterValue | undefined): number | null {
  return typeof value === 'number' ? value : null
}

function booleanValue(value: ParameterValue | undefined): boolean {
  return value === true
}

function stringValue(value: ParameterValue | undefined): string {
  return typeof value === 'string' ? value : ''
}

function setParameter(name: string, value: ParameterValue) {
  parameters[name] = value
  if (!modelId.value) return
  const memory = readParameterMemory()
  memory[modelId.value] = Object.fromEntries(
    Object.entries(parameters).filter(
      ([key, currentValue]) => schema.value[key] && acceptsParameterValue(schema.value[key]!, currentValue),
    ),
  )
  try {
    localStorage.setItem(parameterMemoryKey, JSON.stringify(memory))
  } catch {
    // Keep generation usable when browser storage is unavailable.
  }
}

function readParameterMemory(): ParameterMemory {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(parameterMemoryKey) ?? '{}')
    return parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as ParameterMemory
      : {}
  } catch {
    return {}
  }
}

function acceptsParameterValue(definition: ParameterDefinition, value: ParameterValue) {
  if (definition.type === 'boolean') return typeof value === 'boolean'
  if (definition.type === 'string') return typeof value === 'string'
  if (definition.type === 'enum') {
    return typeof value === 'string' && (definition.allow_custom === true || definition.options?.includes(value) === true)
  }
  if (typeof value !== 'number' || !Number.isFinite(value)) return false
  if (definition.type === 'integer' && !Number.isInteger(value)) return false
  return (definition.min === undefined || value >= definition.min)
    && (definition.max === undefined || value <= definition.max)
}

function parameterLabel(name: string) {
  const labels: Record<string, string> = {
    output_format: '输出格式',
    output_compression: '输出压缩（JPEG/WebP）',
    background: '背景模式',
    moderation: '内容审核级别',
    partial_images: '流式局部预览数量',
    input_fidelity: '输入图片保真度',
    response_format: '上游返回格式',
    style: 'DALL·E 原生风格',
    resolution: '原生分辨率',
  }
  return labels[name] ?? name.replace(/_/g, ' ')
}

function stageText(stage: string) {
  const stages: Record<string, string> = {
    retrying: '正在重新生成',
    automatic_retry: '正在自动重试',
    'provider.processing': '模型正在生成',
    'provider.downloading': '正在接收生成结果',
    'storage.validating': '正在校验图片',
    'storage.persisting': '正在保存原图',
  }
  return stages[stage] ?? '正在处理'
}

function beginNewTemplate() {
  editingTemplateId.value = null
  templateTitle.value = ''
  templatePrompt.value = ''
}

function editTemplate(template: PromptTemplate) {
  if (!template.ownerId) return
  editingTemplateId.value = template.id
  templateTitle.value = template.title
  templatePrompt.value = template.prompt
}

function openTemplateManager(template?: PromptTemplate) {
  if (template?.ownerId) editTemplate(template)
  else beginNewTemplate()
  templateManagerOpen.value = true
}

async function saveTemplate() {
  if (!templateTitle.value.trim() || !templatePrompt.value.trim()) return message.error('请填写模板名称和 Prompt')
  if (editingTemplateId.value) {
    await api(`/api/v1/prompt-templates/${editingTemplateId.value}`, {
      method: 'PATCH',
      body: JSON.stringify({ title: templateTitle.value, prompt: templatePrompt.value }),
    })
  } else {
    await api('/api/v1/prompt-templates', {
      method: 'POST',
      body: JSON.stringify({ templateType: 'style', title: templateTitle.value, prompt: templatePrompt.value }),
    })
  }
  await loadTemplates()
  templateManagerOpen.value = false
  message.success('模板已保存')
}

async function scrollBottom() {
  await nextTick()
  timeline.value?.scrollTo({ top: timeline.value.scrollHeight, behavior: 'smooth' })
}
</script>

<template>
  <div class="studio-shell">
    <aside
      id="studio-conversations"
      class="conversation-rail"
      :class="{ open: mobilePanel === 'conversations' }"
    >
      <div class="rail-heading">
        <div><span class="eyebrow muted">WORKSPACE</span><h2>创作会话</h2></div>
        <div class="panel-heading-actions">
          <button class="icon-button" title="新建会话" @click="createConversation">＋</button>
          <button class="studio-panel-close" type="button" aria-label="关闭会话列表" @click="mobilePanel = null">×</button>
        </div>
      </div>
      <n-input v-model:value="conversationSearch" clearable placeholder="搜索会话标题" size="small" />
      <div class="conversation-list">
        <article
          v-for="(item, index) in filteredConversations"
          :key="item.id"
          class="conversation-item"
          :class="{ active: item.id === activeConversation?.id }"
          :draggable="!conversationSearch.trim()"
          @dragstart="draggedIndex = index"
          @dragover.prevent
          @drop="dropConversation(index)"
          @click="selectConversation(item.id)"
        >
          <span class="drag-handle">⠿</span>
          <div v-if="editingTitle === item.id" class="title-editor" @click.stop>
            <input v-model="titleDraft" autofocus @keyup.enter="saveTitle(item)" @keyup.esc="editingTitle = null" @blur="saveTitle(item)" />
          </div>
          <div v-else class="conversation-copy">
            <strong>{{ item.title }}</strong>
            <small><span v-if="conversationTaskStates[item.id]?.sending" class="conversation-generating">生成中 · </span>{{ new Date(item.lastMessageAt).toLocaleString() }}</small>
          </div>
          <button class="edit-title" title="修改标题" @click.stop="beginTitle(item)">✎</button>
        </article>
        <div v-if="!filteredConversations.length" class="empty-compact">
          {{ conversations.length ? '没有匹配的会话标题。' : '还没有会话，点击右上角开始。' }}
        </div>
      </div>
    </aside>

    <section class="studio-main">
      <header class="studio-header">
        <div><span class="eyebrow muted">CREATIVE SESSION</span><h1>{{ activeConversation?.title || '开始新的创作' }}</h1></div>
        <div class="studio-mobile-tools">
          <button
            class="conversation-panel-trigger"
            type="button"
            aria-controls="studio-conversations"
            :aria-expanded="mobilePanel === 'conversations'"
            @click="mobilePanel = 'conversations'"
          >☰ 会话</button>
          <button
            class="parameter-panel-trigger"
            type="button"
            aria-controls="studio-parameters"
            :aria-expanded="mobilePanel === 'parameters'"
            @click="mobilePanel = 'parameters'"
          >⚙ 参数</button>
        </div>
        <div class="task-status-actions">
          <span v-if="taskStage" class="task-pill" :class="{ active: sending && !cancelling }">{{ taskStatusWithElapsed }}</span>
          <button
            v-if="sending && activeTaskId"
            type="button"
            class="cancel-task-button"
            :disabled="cancelling"
            @click="cancelActiveTask"
          >{{ cancelling ? '取消中' : '取消生成' }}</button>
        </div>
      </header>
      <div ref="timeline" class="message-timeline">
        <div v-if="!visibleMessages.length && !visiblePendingUserMessage" class="studio-empty">
          <div class="empty-art">✦</div>
          <h2>描述你脑海中的画面</h2>
          <p>可以连续追问“保持人物，把背景换成雨夜”。相关历史图片会由系统按当前会话自动选择。</p>
        </div>
        <article v-for="item in visibleMessages" :key="item.id" class="message-row" :class="item.role">
          <div class="message-avatar">{{ item.role === 'user' ? '你' : 'AI' }}</div>
          <div class="message-body">
            <p v-if="item.content && item.status !== 'failed'">{{ item.content }}</p>
            <div v-if="item.status === 'failed'" class="task-error">
              <div>
                <strong>生成失败</strong>
                <span>{{ item.taskErrorMessage || item.content || '任务执行失败，请稍后重试。' }}</span>
              </div>
              <button
                v-if="item.taskId"
                type="button"
                :disabled="sending"
                @click="retryTask(item)"
              >{{ retryingTaskId === item.taskId ? '重试中' : '重试' }}</button>
            </div>
            <div v-if="item.status === 'streaming'" class="streaming-line"><i></i><span>{{ taskStatusWithElapsed }}</span></div>
            <div v-if="visibleMessageAssets(item).length" class="message-images">
              <figure v-for="asset in visibleMessageAssets(item)" :key="asset.id" class="message-image-item">
                <button
                  type="button"
                  class="message-image-button"
                  :aria-label="`放大${messageImageLabel(item)}`"
                  @click="openImagePreview(asset, messageImageLabel(item))"
                >
                  <img :src="asset.contentUrl" :alt="`${messageImageLabel(item)} ${asset.id}`" />
                </button>
                <figcaption class="message-image-meta">
                  <span>{{ asset.width }} × {{ asset.height }} · {{ asset.mimeType }}</span>
                  <span class="image-asset-actions">
                    <button
                      v-if="item.role === 'assistant'"
                      type="button"
                      class="image-edit-button"
                      @click="openImagePreview(asset, messageImageLabel(item), 'crop')"
                    >裁剪缩放</button>
                    <a
                      class="image-download-button"
                      :href="asset.contentUrl"
                      :download="imageDownloadName(asset.id, asset.mimeType)"
                    >↓ 下载原图</a>
                  </span>
                </figcaption>
              </figure>
            </div>
            <time v-if="item.status !== 'streaming'" class="message-time" :datetime="item.taskFinishedAt || item.createdAt">{{ messageTimeText(item) }}</time>
            <div v-if="messageBranch(item).total > 1 || item.role === 'assistant'" class="message-footer">
              <div v-if="messageBranch(item).total > 1" class="branch-switcher" aria-label="消息分支切换">
                <button
                  type="button"
                  aria-label="上一个分支"
                  :disabled="messageBranch(item).index === 0"
                  @click="selectSiblingBranch(item, -1)"
                >‹</button>
                <span>分支 {{ messageBranch(item).index + 1 }} / {{ messageBranch(item).total }}</span>
                <button
                  type="button"
                  aria-label="下一个分支"
                  :disabled="messageBranch(item).index === messageBranch(item).total - 1"
                  @click="selectSiblingBranch(item, 1)"
                >›</button>
              </div>
              <div v-if="item.role === 'assistant'" class="message-actions">
                <button
                  v-if="item.id !== visibleLeafId && item.status !== 'streaming'"
                  type="button"
                  :disabled="sending"
                  @click="continueFrom(item)"
                >从这里继续</button>
                <button
                  v-if="canRegenerate(item)"
                  type="button"
                  :disabled="sending"
                  @click="regenerate(item)"
                >重新生成</button>
              </div>
            </div>
          </div>
        </article>
        <article v-if="visiblePendingUserMessage" class="message-row user pending-message">
          <div class="message-avatar">你</div>
          <div class="message-body">
            <p>{{ visiblePendingUserMessage.content }}</p>
            <time class="message-time" :datetime="visiblePendingUserMessage.createdAt">
              发送 {{ formatMessageTime(visiblePendingUserMessage.createdAt) }}
            </time>
          </div>
        </article>
        <article v-if="sending && !retryingTaskId && visiblePendingUserMessage" class="message-row assistant pending-message">
          <div class="message-avatar">AI</div>
          <div class="message-body">
            <div class="streaming-line"><i></i><span>{{ taskStatusWithElapsed }}</span></div>
            <div v-if="partialPreview" class="message-images">
              <button
                type="button"
                class="message-image-button"
                :aria-label="`放大${partialPreview.label}`"
                @click="openPartialPreview(partialPreview.contentUrl, partialPreview.label)"
              >
                <img :src="partialPreview.contentUrl" :alt="partialPreview.label" />
                <span>{{ partialPreview.label }} · 最终原图仍在生成</span>
              </button>
            </div>
          </div>
        </article>
      </div>
      <footer class="composer-wrap">
        <div v-if="composerAnchor" class="branch-anchor">
          <span><strong>从这里继续</strong> · {{ composerAnchor.content || '这条图片结果' }}</span>
          <button type="button" aria-label="取消从这里继续" @click="resetComposerBranch">取消</button>
        </div>
        <div class="composer">
          <div v-if="files.length" class="composer-attachments" aria-label="待发送参考图">
            <figure
              v-for="(attachment, index) in files"
              :key="attachment.previewUrl"
              class="composer-attachment"
              :title="attachment.file.name"
            >
              <img :src="attachment.previewUrl" :alt="`参考图 ${attachment.file.name}`" />
              <button type="button" :aria-label="`移除参考图 ${attachment.file.name}`" @click="removeFile(index)">×</button>
            </figure>
          </div>
          <textarea ref="composerInput" v-model="prompt" rows="2" placeholder="继续描述你的画面，或基于上一轮提出修改…" @paste="pasteFiles" @keydown.ctrl.enter.prevent="send"></textarea>
          <div class="composer-actions">
            <label class="upload-button">＋ 参考图<input ref="fileInput" type="file" accept="image/png,image/jpeg,image/webp" multiple @change="chooseFiles" /></label>
            <span class="composer-shortcut">Ctrl + Enter 发送</span>
            <button class="send-button" :disabled="sending || !prompt.trim()" @click="send">{{ sending ? '生成中' : '生成' }} <b>↗</b></button>
          </div>
        </div>
      </footer>
    </section>

    <aside
      id="studio-parameters"
      class="parameter-panel"
      :class="{ resizing: resizingParameterPanel, open: mobilePanel === 'parameters' }"
    >
      <div
        class="parameter-resize-handle"
        role="separator"
        aria-label="调整生成参数栏宽度"
        aria-orientation="vertical"
        :aria-valuemin="parameterPanelMinWidth"
        :aria-valuemax="parameterPanelMaxWidth"
        :aria-valuenow="parameterPanelWidth"
        tabindex="0"
        title="拖动调整生成参数栏宽度"
        @pointerdown="startParameterResize"
        @keydown.left.prevent="resizeParameterPanel(40)"
        @keydown.right.prevent="resizeParameterPanel(-40)"
      ><span></span></div>
      <header>
        <div><span class="eyebrow muted">PARAMETERS</span><h2>生成参数</h2></div>
        <button class="studio-panel-close" type="button" aria-label="关闭生成参数" @click="mobilePanel = null">×</button>
      </header>
      <div class="parameter-scroll">
        <section class="parameter-group model-parameters">
          <h3>模型</h3>
          <label>Provider<n-select v-model:value="providerId" :options="providerOptions" placeholder="选择 Provider" /></label>
          <label>生图模型<n-select v-model:value="modelId" :options="modelOptions" placeholder="选择已验证模型" /></label>
        </section>
        <section class="parameter-group base-parameters">
          <h3>基础参数</h3>
          <image-size-control
            v-if="schema.size"
            :aspect-ratio="stringValue(parameters.aspect_ratio) || 'auto'"
            :size="stringValue(parameters.size) || 'auto'"
            :aspect-definition="schema.aspect_ratio"
            :size-definition="schema.size"
            @update:aspect-ratio="value => setParameter('aspect_ratio', value)"
            @update:size="value => setParameter('size', value)"
          />
          <label v-if="schema.quality">质量<n-select :value="selectValue(parameters.quality)" :options="qualityOptions" @update:value="value => setParameter('quality', value)" /></label>
          <label v-if="schema.n">生成数量<n-input-number :value="numberValue(parameters.n)" :min="schema.n.min ?? 1" :max="schema.n.max ?? 10" @update:value="value => setParameter('n', value)" /></label>
          <div class="parameter-field"><span>创作风格</span>
            <div class="inline-control"><n-select v-model:value="styleId" clearable :options="styleOptions" /><button @click="openTemplateManager(currentTemplate ?? undefined)">管理模板</button></div>
          </div>
        </section>
        <details class="parameter-group advanced" open>
          <summary>高级设置 <span>{{ advancedParameters.length }} 项</span></summary>
          <div class="advanced-parameter-grid">
            <div v-if="!advancedParameters.length" class="schema-empty">当前模型没有声明更多高级参数。</div>
            <label v-for="([name, definition]) in advancedParameters" :key="name">
              {{ parameterLabel(name) }}
              <n-select v-if="definition.type === 'enum'" :value="selectValue(parameters[name])" :options="parameterOptions(definition)" @update:value="value => setParameter(name, value)" />
              <n-input-number v-else-if="definition.type === 'integer' || definition.type === 'number'" :value="numberValue(parameters[name])" :min="definition.min" :max="definition.max" :step="definition.step" @update:value="value => setParameter(name, value)" />
              <n-switch v-else-if="definition.type === 'boolean'" :value="booleanValue(parameters[name])" @update:value="value => setParameter(name, value)" />
              <n-input v-else :value="stringValue(parameters[name])" @update:value="value => setParameter(name, value)" />
            </label>
          </div>
        </details>
      </div>
    </aside>
    <button
      v-if="mobilePanel"
      class="studio-panel-backdrop"
      type="button"
      :aria-label="mobilePanel === 'conversations' ? '关闭会话列表' : '关闭生成参数'"
      @click="mobilePanel = null"
    ></button>
  </div>

  <n-modal v-model:show="templateManagerOpen" preset="card" title="管理创作风格模板" class="dialog-card">
    <div class="template-layout">
      <div class="template-list">
        <button :class="{ active: !editingTemplateId }" @click="beginNewTemplate">＋ 新建模板</button>
        <button v-for="item in templates" :key="item.id" :class="{ active: editingTemplateId === item.id }" :disabled="!item.ownerId" @click="editTemplate(item)">
          {{ item.title }} <small>{{ item.ownerId ? '我的模板' : '系统模板' }}</small>
        </button>
      </div>
      <div class="form-stack">
        <div class="template-editor-heading">
          <strong>{{ editingTemplateId ? '编辑我的模板' : '新建模板' }}</strong>
          <span>{{ editingTemplateId ? '修改后将覆盖当前模板。' : '填写名称和 Prompt，保存后即可在创作风格中选择。' }}</span>
        </div>
        <label>模板名称<n-input v-model:value="templateTitle" placeholder="例如：复古胶片" /></label>
        <label>Prompt 文本<n-input v-model:value="templatePrompt" type="textarea" :rows="7" /></label>
        <n-button type="primary" @click="saveTemplate">{{ editingTemplateId ? '保存修改' : '创建模板' }}</n-button>
      </div>
    </div>
  </n-modal>

  <image-crop-modal
    v-model:show="imagePreviewOpen"
    :image="imagePreview"
    :initial-mode="imagePreviewMode"
  />
</template>
