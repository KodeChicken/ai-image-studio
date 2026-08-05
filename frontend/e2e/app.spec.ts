import { expect, test, type Page } from '@playwright/test'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { deflateSync } from 'node:zlib'

const user = {
  id: '00000000-0000-4000-8000-000000000001',
  username: 'admin',
  displayName: '系统管理员',
  role: 'admin',
  mustChangePassword: false,
  themePreference: 'system',
}

const provider = {
  id: '10000000-0000-4000-8000-000000000001',
  providerKey: 'openai',
  providerType: 'openai-compatible',
  displayName: 'OpenAI Compatible',
  baseUrl: 'https://api.openai.com/v1',
  enabled: true,
  configJson: {},
  credentialConfigured: true,
  modelCount: 1,
  healthStatus: 'healthy',
  lastHealthCheckedAt: '2026-07-21T10:00:00Z',
  lastHealthError: null,
  createdAt: '2026-07-21T10:00:00Z',
  updatedAt: '2026-07-21T10:00:00Z',
}

const model = {
  id: '20000000-0000-4000-8000-000000000001',
  providerId: provider.id,
  providerType: provider.providerType,
  modelKey: 'gpt-image-1',
  upstreamModelId: 'gpt-image-1',
  displayName: 'GPT Image 1',
  capabilities: { text_to_image: true },
  parameterSchema: {
    parameters: {
      size: { type: 'enum', default: 'auto', options: ['auto', '1024x1024'] },
      quality: { type: 'enum', default: 'auto', options: ['auto', 'high'] },
      n: { type: 'integer', default: 1, min: 1, max: 4 },
      background: { type: 'enum', default: 'auto', options: ['auto', 'transparent', 'opaque'] },
      moderation: { type: 'enum', default: 'auto', options: ['auto', 'low'] },
      output_format: { type: 'enum', default: 'png', options: ['png', 'jpeg', 'webp'] },
      output_compression: {
        type: 'integer',
        default: 100,
        min: 0,
        max: 100,
        visible_when: { output_format: ['jpeg', 'webp'] },
      },
      partial_images: {
        type: 'integer',
        default: 0,
        min: 0,
        max: 3,
        visible_when: { stream: true },
      },
      input_fidelity: {
        type: 'enum',
        default: 'low',
        options: ['low', 'high'],
        operations: ['edit'],
      },
    },
  },
  availabilityStatus: 'verified',
  discoverySource: 'upstream_list',
  capabilitySource: 'official_catalog',
  lastDiscoveredAt: '2026-07-21T10:00:00Z',
  lastVerifiedAt: '2026-07-21T10:00:00Z',
  enabled: true,
}

const geminiModel = {
  ...model,
  id: '20000000-0000-4000-8000-000000000002',
  providerType: 'gemini',
  modelKey: 'gemini-2.5-flash-image',
  upstreamModelId: 'gemini-2.5-flash-image',
  displayName: 'Gemini Flash Image',
  parameterSchema: {
    parameters: {
      aspect_ratio: {
        type: 'enum',
        default: 'auto',
        options: ['auto', '1:1', '16:9', '21:9'],
      },
      size: { type: 'enum', default: 'auto', options: ['auto', '1k', '2k', '4k'] },
      n: { type: 'integer', default: 1, min: 1, max: 1 },
    },
  },
}

const gptImage2Model = {
  ...model,
  id: '20000000-0000-4000-8000-000000000004',
  modelKey: 'gpt-image-2',
  upstreamModelId: 'gpt-image-2',
  displayName: 'GPT Image 2',
  parameterSchema: {
    parameters: {
      aspect_ratio: {
        type: 'enum',
        default: 'auto',
        options: ['auto', '1:1', '3:2', '2:3', '16:9', '9:16'],
      },
      size: {
        type: 'enum',
        default: 'auto',
        options: ['auto', '1024x1024', '3840x2160', '2160x3840'],
        allow_custom: true,
        constraints: {
          edgeMultiple: 16,
          maxEdge: 3840,
          minPixels: 655360,
          maxPixels: 8294400,
          maxAspectRatio: 3,
        },
      },
      quality: { type: 'enum', default: 'auto', options: ['auto', 'high'] },
      n: { type: 'integer', default: 1, min: 1, max: 4 },
    },
  },
}

const textModel = {
  ...model,
  id: '20000000-0000-4000-8000-000000000003',
  modelKey: 'gpt-5.4',
  upstreamModelId: 'gpt-5.4',
  displayName: 'GPT-5.4',
  capabilities: {},
  parameterSchema: { parameters: {} },
  availabilityStatus: 'discovered',
  capabilitySource: 'probe',
}

const modelPrice = {
  id: '21000000-0000-4000-8000-000000000001',
  modelId: model.id,
  pricingType: 'image',
  dimensionKey: 'image',
  price: '0.040000',
  currency: 'USD',
  effectiveFrom: '2026-07-21T10:00:00Z',
  effectiveTo: null,
  createdAt: '2026-07-21T10:00:00Z',
}

const conversation = {
  id: '30000000-0000-4000-8000-000000000001',
  title: '雨夜霓虹人物海报',
  status: 'active',
  defaultProviderId: provider.id,
  defaultModelId: model.id,
  sortOrder: 1024,
  lastMessageAt: '2026-07-21T10:00:00Z',
  createdAt: '2026-07-21T10:00:00Z',
  updatedAt: '2026-07-21T10:00:00Z',
}

const secondConversation = {
  ...conversation,
  id: '30000000-0000-4000-8000-000000000002',
  title: '极简香水产品摄影',
  sortOrder: 2048,
}

const consistencyRun = {
  id: '40000000-0000-4000-8000-000000000001',
  status: 'succeeded',
  deleteOrphans: false,
  graceSeconds: 86400,
  databaseAssets: 12,
  storageObjects: 13,
  missingObjects: 0,
  orphanObjects: 1,
  eligibleOrphans: 0,
  deletedOrphans: 0,
  errorMessage: null,
  requestedBy: user.id,
  startedAt: '2026-07-21T10:00:00Z',
  finishedAt: '2026-07-21T10:00:01Z',
}

const systemStyleTemplate = {
  id: '50000000-0000-4000-8000-000000000001',
  ownerId: null,
  templateType: 'style',
  title: '电影感',
  applicableScenarios: '叙事海报、角色场景和需要戏剧氛围的画面',
  prompt: 'cinematic lighting',
  negativePrompt: null,
  tags: ['cinematic'],
  isPublic: true,
  enabled: true,
}

const digitalInternetStylePrompt = '互联网行业大会海报风格，蓝色为主色调，深蓝到亮蓝渐变背景，现代科技感与未来科幻氛围，采用数字互联网和大数据视觉语言；融入流动数据网络、发光科技线条、抽象网格、HUD 界面、数字节点、科幻粒子和光点，结合清晰的流程信息可视化布局；整体呈现企业级、专业、高端、简洁的设计质感，具有强视觉中心与明确的信息层级，预留醒目的大会标题、主题文案和关键信息排版区域，画面精致通透、富有空间纵深，适用于互联网行业大会、数字峰会及流程海报。保持用户指定的主体、内容和构图要求。'
const digitalInternetApplicableScenarios = '互联网行业大会、数字峰会、科技发布会、流程海报和大数据主题视觉'

const digitalInternetStyleTemplate = {
  id: '50000000-0000-4000-8000-000000000002',
  ownerId: null,
  templateType: 'style',
  title: '数字互联网大会',
  applicableScenarios: digitalInternetApplicableScenarios,
  prompt: digitalInternetStylePrompt,
  negativePrompt: null,
  tags: ['互联网风格', '行业大会', '流程海报', '科技感', '蓝色', '科幻', '科幻粒子', '大数据', '数字互联网'],
  isPublic: true,
  enabled: true,
}

function solidGrayscalePng(width: number, height: number) {
  const scanlines = Buffer.alloc((width + 1) * height, 0xdd)
  for (let row = 0; row < height; row += 1) scanlines[row * (width + 1)] = 0
  const header = Buffer.alloc(13)
  header.writeUInt32BE(width, 0)
  header.writeUInt32BE(height, 4)
  header.set([8, 0, 0, 0, 0], 8)
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(scanlines)),
    pngChunk('IEND', Buffer.alloc(0)),
  ])
}

function pngChunk(type: string, data: Buffer) {
  const typeBytes = Buffer.from(type, 'ascii')
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const checksum = Buffer.alloc(4)
  checksum.writeUInt32BE(crc32(Buffer.concat([typeBytes, data])))
  return Buffer.concat([length, typeBytes, data, checksum])
}

function crc32(data: Buffer) {
  let crc = 0xffffffff
  for (const byte of data) {
    crc ^= byte
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1))
  }
  return (crc ^ 0xffffffff) >>> 0
}

async function mockApi(
  page: Page,
  initiallyAuthenticated: boolean,
  options: {
    disconnectEventStreamOnce?: boolean
    multipleModels?: boolean
    customSizeModel?: boolean
    cancellableTask?: boolean
    retryableTask?: boolean
    rejectedTask?: boolean
    messageCreationFailure?: boolean
    multipleConversations?: boolean
    automaticReferenceInHistory?: boolean
    authUser?: typeof user
    includeTextModel?: boolean
    slowTask?: boolean
    editorSourceWidth?: number
    editorSourceHeight?: number
  } = {},
) {
  let authenticated = initiallyAuthenticated
  let themePreference = 'system'
  const authUser = { ...(options.authUser ?? user) }
  let uploadSequence = 0
  let messageSequence = 0
  let eventStreamDisconnected = false
  let createdConversation: typeof conversation | null = null
  let editorSourceAssetId = '73000000-0000-4000-8000-000000000099'
  let editorVersion = 1
  const editorSourceWidth = options.editorSourceWidth ?? 1024
  const editorSourceHeight = options.editorSourceHeight ?? 1024
  const editorSourcePng = solidGrayscalePng(editorSourceWidth, editorSourceHeight)
  let editorDocument = {
    schemaVersion: 1 as const,
    canvas: { width: editorSourceWidth, height: editorSourceHeight, background: { type: 'transparent' as const } },
    layout: { fitStrategy: 'cover', anchor: 'center' },
    image: {
      assetId: '73000000-0000-4000-8000-000000000099',
      x: 0, y: 0, scaleX: 1, scaleY: 1, rotation: 0, flipX: false, flipY: false,
      crop: { x: 0, y: 0, width: editorSourceWidth, height: editorSourceHeight },
    },
  }
  const messages: Array<Record<string, unknown>> = []
  const taskAssistantIds = new Map<string, string>()
  const cancelledTaskIds = new Set<string>()
  const retriedTaskIds = new Set<string>()
  const promptTemplates: Array<Record<string, unknown>> = [systemStyleTemplate, digitalInternetStyleTemplate]
  const usageRecords = Array.from({ length: 75 }, (_, index) => ({
    id: 75 - index,
    taskId: `70000000-0000-4000-8000-${String(75 - index).padStart(12, '0')}`,
    providerName: provider.displayName,
    modelName: model.displayName,
    quantity: 1,
    unit: 'image',
    cost: 0.125,
    currency: 'CNY',
    createdAt: '2026-07-21T10:00:00Z',
  }))
  const state = {
    messageRequests: [] as Array<Record<string, unknown>>,
    messageConversationIds: [] as string[],
    uploadedAssetIds: [] as string[],
    resumeEventIds: [] as string[],
    templateWrites: [] as Array<Record<string, unknown>>,
    themePreferences: [] as string[],
    logoutRequests: 0,
    cancelRequests: [] as string[],
    retryRequests: [] as string[],
    assetDeleteRequests: [] as string[],
    historyQueries: [] as Array<Record<string, string>>,
    passwordRequests: [] as Array<Record<string, unknown>>,
    providerWrites: [] as Array<Record<string, unknown>>,
    testGenerationRequests: [] as Array<Record<string, unknown>>,
    editorSaves: [] as Array<typeof editorDocument>,
    editorExports: [] as Array<{ width: number; height: number; format: string }>,
  }
  await page.route('**/mock/generated.png', (route) => route.fulfill({
    status: 200,
    contentType: 'image/svg+xml',
    body: '<svg xmlns="http://www.w3.org/2000/svg" width="600" height="900"><rect width="600" height="900" fill="#ddd4c8"/><circle cx="300" cy="170" r="90" fill="#9b7e6b"/><rect x="165" y="285" width="270" height="500" rx="80" fill="#f4efe9"/></svg>',
  }))
  await page.route('**/mock/editor-source.png', (route) => route.fulfill({
    status: 200,
    contentType: 'image/png',
    body: editorSourcePng,
  }))
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    const path = url.pathname
    const method = request.method()
    const fulfill = (json: unknown, status = 200) => route.fulfill({ status, json })

    if (path === '/api/v1/users/me' && method === 'GET') {
      return authenticated
        ? fulfill({ ...authUser, themePreference })
        : fulfill({ error: { code: 'UNAUTHORIZED', message: 'authentication required' } }, 401)
    }
    if (path === '/api/v1/auth/login' && method === 'POST') {
      authenticated = true
      return fulfill({ ...authUser, themePreference })
    }
    if (path === '/api/v1/auth/logout' && method === 'POST') {
      authenticated = false
      state.logoutRequests += 1
      return route.fulfill({ status: 204 })
    }
    if (path === '/api/v1/users/me/preferences' && method === 'PATCH') {
      themePreference = String(request.postDataJSON().themePreference)
      state.themePreferences.push(themePreference)
      return fulfill({ ...authUser, themePreference })
    }
    if (path === '/api/v1/users/me/change-password' && method === 'POST') {
      state.passwordRequests.push(request.postDataJSON() as Record<string, unknown>)
      authUser.mustChangePassword = false
      return route.fulfill({ status: 204 })
    }
    if (path === '/api/v1/conversations' && method === 'GET') {
      const items = options.multipleConversations ? [conversation, secondConversation] : [conversation]
      return fulfill(createdConversation ? [...items, createdConversation] : items)
    }
    if (path === '/api/v1/conversations' && method === 'POST') {
      const input = request.postDataJSON() as Record<string, unknown>
      createdConversation = {
        ...conversation,
        id: '30000000-0000-4000-8000-000000000099',
        title: String(input.title),
        defaultProviderId: typeof input.defaultProviderId === 'string' ? input.defaultProviderId : provider.id,
        defaultModelId: typeof input.defaultModelId === 'string' ? input.defaultModelId : model.id,
        lastMessageAt: '2026-07-22T10:27:40Z',
        createdAt: '2026-07-22T10:27:40Z',
        updatedAt: '2026-07-22T10:27:40Z',
      }
      return fulfill(createdConversation, 201)
    }
    if (path === `/api/v1/conversations/${conversation.id}` && method === 'GET') {
      return fulfill({
        ...conversation,
        messages: messages.filter((item) => item.conversationId === conversation.id),
      })
    }
    if (path === `/api/v1/conversations/${secondConversation.id}` && method === 'GET') {
      return fulfill({
        ...secondConversation,
        defaultModelId: options.multipleModels ? geminiModel.id : model.id,
        messages: messages.filter((item) => item.conversationId === secondConversation.id),
      })
    }
    if (createdConversation && path === `/api/v1/conversations/${createdConversation.id}` && method === 'GET') {
      return fulfill({
        ...createdConversation,
        messages: messages.filter((item) => item.conversationId === createdConversation!.id),
      })
    }
    if (path === '/api/v1/providers' && method === 'GET') return fulfill([provider])
    if (path === '/api/v1/providers' && method === 'POST') {
      const input = request.postDataJSON() as Record<string, unknown>
      state.providerWrites.push(input)
      return fulfill({
        ...provider,
        id: crypto.randomUUID(),
        providerKey: input.providerKey,
        providerType: input.providerType,
        displayName: input.displayName,
        baseUrl: input.baseUrl,
        credentialConfigured: Boolean(input.apiKey),
      }, 201)
    }
    if (path === '/api/v1/models' && method === 'GET') {
      return fulfill([
        model,
        ...(options.multipleModels ? [geminiModel] : []),
        ...(options.customSizeModel ? [gptImage2Model] : []),
        ...(options.includeTextModel ? [textModel] : []),
      ])
    }
    if (path === '/api/v1/prompt-templates' && method === 'GET') return fulfill(promptTemplates)
    if (path === '/api/v1/prompt-templates' && method === 'POST') {
      const input = request.postDataJSON() as Record<string, unknown>
      const created = {
        id: `50000000-0000-4000-8000-${String(promptTemplates.length + 1).padStart(12, '0')}`,
        ownerId: user.id,
        negativePrompt: null,
        tags: [],
        isPublic: false,
        enabled: true,
        ...input,
      }
      promptTemplates.push(created)
      state.templateWrites.push(input)
      return fulfill(created, 201)
    }
    const promptTemplateMatch = path.match(/^\/api\/v1\/prompt-templates\/([0-9a-f-]+)$/)
    if (promptTemplateMatch && method === 'PATCH') {
      const template = promptTemplates.find((item) => item.id === promptTemplateMatch[1] && item.ownerId === user.id)
      if (!template) return fulfill({ error: { code: 'NOT_FOUND', message: 'template not found' } }, 404)
      const input = request.postDataJSON() as Record<string, unknown>
      Object.assign(template, input)
      state.templateWrites.push(input)
      return fulfill(template)
    }
    if (path === '/api/v1/image-assets/uploads' && method === 'POST') {
      uploadSequence += 1
      const id = `60000000-0000-4000-8000-${String(uploadSequence).padStart(12, '0')}`
      state.uploadedAssetIds.push(id)
      return fulfill({
        id,
        contentUrl: `/api/v1/image-assets/${id}/content`,
        mimeType: 'image/png',
        width: 1,
        height: 1,
        fileSizeBytes: 68,
      }, 201)
    }
    const assetDeleteMatch = path.match(/^\/api\/v1\/image-assets\/([0-9a-f-]+)$/)
    if (assetDeleteMatch && method === 'DELETE') {
      state.assetDeleteRequests.push(assetDeleteMatch[1]!)
      return route.fulfill({ status: 204 })
    }
    const conversationMessageMatch = path.match(/^\/api\/v1\/conversations\/([0-9a-f-]+)\/messages$/)
    if (conversationMessageMatch && method === 'POST') {
      const requestConversationId = conversationMessageMatch[1]!
      messageSequence += 1
      const input = request.postDataJSON() as Record<string, unknown>
      state.messageRequests.push(input)
      state.messageConversationIds.push(requestConversationId)
      if (options.messageCreationFailure) {
        return fulfill({ error: { code: 'VALIDATION_ERROR', message: '任务参数校验失败' } }, 400)
      }
      const taskId = `70000000-0000-4000-8000-${String(messageSequence).padStart(12, '0')}`
      const userMessageId = `71000000-0000-4000-8000-${String(messageSequence).padStart(12, '0')}`
      const assistantMessageId = `72000000-0000-4000-8000-${String(messageSequence).padStart(12, '0')}`
      taskAssistantIds.set(taskId, assistantMessageId)
      const parentMessageId = typeof input.parentMessageId === 'string' ? input.parentMessageId : null
      const inputAssetIds = Array.isArray(input.inputAssetIds)
        ? input.inputAssetIds.filter((value): value is string => typeof value === 'string')
        : []
      const shouldReject = options.rejectedTask === true && messageSequence === 1
      const shouldFail = (options.retryableTask === true || shouldReject) && messageSequence === 1
      const taskErrorCode = shouldReject ? 'moderation_blocked' : 'UPSTREAM_UNAVAILABLE'
      const taskErrorMessage = shouldReject
        ? '图片生成请求未通过安全检查，请调整提示词或输入图片后重试。'
        : '上游服务暂时不可用'
      messages.push(
        {
          id: userMessageId,
          conversationId: requestConversationId,
          parentMessageId,
          role: 'user',
          status: 'completed',
          sequenceNo: messageSequence * 2 - 1,
          content: input.content,
          metadata: {},
          taskId: null,
          taskErrorCode: null,
          taskErrorMessage: null,
          taskRetryCount: null,
          taskStartedAt: null,
          taskFinishedAt: null,
          assets: inputAssetIds.map((id) => ({
            id,
            contentUrl: `/api/v1/image-assets/${id}/content`,
            mimeType: 'image/png',
            width: 1,
            height: 1,
            fileSizeBytes: 68,
            relationType: 'attachment',
          })).concat(options.automaticReferenceInHistory && messageSequence > 1 ? [{
            id: `73000000-0000-4000-8000-${String(messageSequence - 1).padStart(12, '0')}`,
            contentUrl: '/mock/generated.png',
            mimeType: 'image/png',
            width: 1024,
            height: 1024,
            fileSizeBytes: 1024,
            relationType: 'reference',
          }] : []),
          createdAt: '2026-07-21T10:00:00Z',
          updatedAt: '2026-07-21T10:00:00Z',
        },
        {
          id: assistantMessageId,
          conversationId: requestConversationId,
          parentMessageId: userMessageId,
          role: 'assistant',
          status: shouldFail ? 'failed' : 'completed',
          sequenceNo: messageSequence * 2,
          content: shouldFail ? '生成失败，请重试' : '已生成 1 张图片',
          metadata: {},
          taskId,
          taskErrorCode: shouldFail ? taskErrorCode : null,
          taskErrorMessage: shouldFail ? taskErrorMessage : null,
          taskRetryCount: 0,
          taskStartedAt: '2026-07-21T10:00:01Z',
          taskFinishedAt: shouldFail ? '2026-07-21T10:01:06Z' : '2026-07-21T10:00:06Z',
          assets: shouldFail ? [] : [{
              id: `73000000-0000-4000-8000-${String(messageSequence).padStart(12, '0')}`,
              contentUrl: '/mock/generated.png',
              mimeType: 'image/png',
              width: 1024,
              height: 1024,
              fileSizeBytes: 1024,
              relationType: 'generated',
            }],
          createdAt: '2026-07-21T10:00:01Z',
          updatedAt: '2026-07-21T10:00:01Z',
        },
      )
      const created = `id: ${messageSequence * 10}\nevent: task.created\ndata: ${JSON.stringify({ taskId })}\n\n`
      const completed = `id: ${messageSequence * 10 + 1}\nevent: task.completed\ndata: ${JSON.stringify({ taskId })}\n\n`
      const failed = `id: ${messageSequence * 10 + 1}\nevent: task.failed\ndata: ${JSON.stringify({ taskId, errorCode: taskErrorCode })}\n\n`
      if (options.slowTask) await new Promise((resolve) => setTimeout(resolve, 4000))
      return route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: shouldFail
          ? created + failed
          : options.disconnectEventStreamOnce || options.cancellableTask
            ? created
            : created + completed,
      })
    }
    const retryMatch = path.match(/^\/api\/v1\/tasks\/([0-9a-f-]+)\/retry$/)
    if (retryMatch && method === 'POST') {
      const taskId = retryMatch[1]!
      retriedTaskIds.add(taskId)
      state.retryRequests.push(taskId)
      const assistant = messages.find((item) => item.id === taskAssistantIds.get(taskId))
      if (assistant) {
        assistant.status = 'streaming'
        assistant.content = null
        assistant.taskErrorCode = null
        assistant.taskErrorMessage = null
        assistant.taskRetryCount = 1
        assistant.taskFinishedAt = null
      }
      return fulfill({ taskId, lastEventId: 500 })
    }
    const cancelMatch = path.match(/^\/api\/v1\/tasks\/([0-9a-f-]+)\/cancel$/)
    if (cancelMatch && method === 'POST') {
      const taskId = cancelMatch[1]!
      cancelledTaskIds.add(taskId)
      state.cancelRequests.push(taskId)
      const assistantMessageId = taskAssistantIds.get(taskId)
      const assistant = messages.find((item) => item.id === assistantMessageId)
      if (assistant) {
        assistant.status = 'cancelled'
        assistant.content = '生成已取消'
        assistant.taskFinishedAt = '2026-07-21T10:00:06Z'
      }
      return route.fulfill({ status: 204 })
    }
    if (path.match(/^\/api\/v1\/tasks\/[0-9a-f-]+\/events$/) && method === 'GET') {
      state.resumeEventIds.push(request.headers()['last-event-id'] ?? '')
      const taskId = path.split('/')[4]
      if (taskId && retriedTaskIds.has(taskId)) {
        const assistant = messages.find((item) => item.id === taskAssistantIds.get(taskId))
        if (assistant) {
          assistant.status = 'completed'
          assistant.content = '已生成 1 张图片'
          assistant.taskFinishedAt = '2026-07-21T10:00:06Z'
          assistant.assets = [{
            id: '73000000-0000-4000-8000-000000000099',
            contentUrl: '/mock/generated.png',
            mimeType: 'image/png',
            width: 1024,
            height: 1024,
            fileSizeBytes: 1024,
          }]
        }
        return route.fulfill({
          status: 200,
          contentType: 'text/event-stream',
          body: `id: 501\nevent: task.progress\ndata: ${JSON.stringify({ taskId, stage: 'provider.processing' })}\n\nid: 502\nevent: task.completed\ndata: ${JSON.stringify({ taskId })}\n\n`,
        })
      }
      if (options.disconnectEventStreamOnce && !eventStreamDisconnected) {
        eventStreamDisconnected = true
        return route.abort('connectionreset')
      }
      if (options.cancellableTask) {
        return route.fulfill({
          status: 200,
          contentType: 'text/event-stream',
          body: cancelledTaskIds.has(taskId!)
            ? `id: 999\nevent: task.cancelled\ndata: ${JSON.stringify({ taskId })}\n\n`
            : '',
        })
      }
      return route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: `id: 999\nevent: task.completed\ndata: ${JSON.stringify({ taskId })}\n\n`,
      })
    }
    if (path === '/api/v1/history' && method === 'GET') {
      state.historyQueries.push(Object.fromEntries(url.searchParams))
      return fulfill([{
        taskId: '70000000-0000-4000-8000-000000000099',
        conversationId: conversation.id,
        conversationTitle: conversation.title,
        assetId: '73000000-0000-4000-8000-000000000099',
        editDocumentId: null,
        contentUrl: '/mock/generated.png',
        modelId: model.id,
        modelName: model.displayName,
        providerId: provider.id,
        providerName: provider.displayName,
        prompt: '历史筛选测试图片',
        mimeType: 'image/png',
        width: 1024,
        height: 1024,
        fileSizeBytes: 1024,
        createdAt: '2026-07-21T10:00:00Z',
      }])
    }
    if (path === '/api/v1/image-edit-documents' && method === 'POST') {
      const input = request.postDataJSON() as { sourceAssetId: string }
      editorSourceAssetId = input.sourceAssetId
      editorVersion = 1
      editorDocument = {
        ...editorDocument,
        image: { ...editorDocument.image, assetId: input.sourceAssetId },
      }
      return fulfill(editorView(), 201)
    }
    if (path === '/api/v1/image-edit-documents/81000000-0000-4000-8000-000000000001' && method === 'GET') {
      return fulfill(editorView())
    }
    if (path === '/api/v1/image-edit-documents/81000000-0000-4000-8000-000000000001' && method === 'PUT') {
      const input = request.postDataJSON() as { document: typeof editorDocument }
      editorDocument = structuredClone(input.document)
      state.editorSaves.push(structuredClone(editorDocument))
      editorVersion += 1
      return fulfill(editorView())
    }
    if (path === '/api/v1/image-edit-documents/81000000-0000-4000-8000-000000000001/exports' && method === 'POST') {
      const body = request.postData() ?? ''
      const format = /name="format"\r\n\r\n([^\r]+)/.exec(body)?.[1] ?? 'png'
      state.editorExports.push({
        width: editorDocument.canvas.width,
        height: editorDocument.canvas.height,
        format,
      })
      return fulfill({
        id: crypto.randomUUID(),
        contentUrl: '/mock/generated.png',
        mimeType: `image/${format}`,
        width: editorDocument.canvas.width,
        height: editorDocument.canvas.height,
        fileSizeBytes: 1024,
      }, 201)
    }
    if (path === '/api/v1/usage' && method === 'GET') {
      const beforeId = Number(url.searchParams.get('beforeId') || Number.MAX_SAFE_INTEGER)
      const limit = Number(url.searchParams.get('limit') || 50)
      const recent = usageRecords.filter((item) => item.id < beforeId).slice(0, limit)
      return fulfill({
        period: { from: '2026-06-21T10:00:00Z', to: '2026-07-21T10:00:00Z' },
        totals: { taskCount: 75, imageCount: 75 },
        costs: [{ currency: 'CNY', totalCost: 9.375 }],
        byModel: [{
          providerId: provider.id,
          modelId: model.id,
          providerName: provider.displayName,
          modelName: model.displayName,
          taskCount: 75,
          imageCount: 75,
          totalCost: 9.375,
          currency: 'CNY',
        }],
        recent,
        nextBeforeId: recent.length === limit ? recent.at(-1)?.id ?? null : null,
      })
    }
    if (path === `/api/v1/providers/${provider.id}/test` && method === 'POST') {
      return fulfill({ status: 'healthy', modelCount: 1, latencyMs: 28, checkedAt: '2026-07-21T10:00:00Z' })
    }
    if (path === `/api/v1/providers/${provider.id}/test-generation` && method === 'POST') {
      const input = request.postDataJSON() as Record<string, unknown>
      state.testGenerationRequests.push(input)
      return fulfill({
        modelId: model.id,
        modelName: model.displayName,
        imageDataUrl: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+X8fZAAAAAElFTkSuQmCC',
        mimeType: 'image/png',
        width: 1024,
        height: 1024,
        latencyMs: 1280,
      })
    }
    if (path === `/api/v1/providers/${provider.id}/models/discover` && method === 'POST') {
      return fulfill({ discovered: 1, verifiedImageModels: 1, models: [model] })
    }
    if (path === `/api/v1/providers/${provider.id}/models/${model.id}` && method === 'POST') {
      return fulfill(model)
    }
    if (path === `/api/v1/providers/${provider.id}/models/${model.id}/pricing` && method === 'GET') {
      return fulfill([modelPrice])
    }
    if (path === '/api/v1/admin/storage' && method === 'GET') {
      return fulfill({
        activeDriver: 'local',
        targetConfig: { driver: 'local', localPath: './data/images' },
        localAssetCount: 12,
        s3AssetCount: 0,
        localPath: './data/images',
        s3Configured: false,
      })
    }
    if (path === '/api/v1/admin/storage/consistency' && method === 'GET') {
      return fulfill([consistencyRun])
    }
    if (path === '/api/v1/admin/storage/consistency/scan' && method === 'POST') {
      return fulfill({ ...consistencyRun, id: crypto.randomUUID() })
    }
    return fulfill({ error: { code: 'NOT_FOUND', message: `unmocked ${method} ${path}` } }, 404)
  })

  function editorView() {
    const asset = {
      id: editorDocument.image.assetId,
      contentUrl: '/mock/editor-source.png',
      mimeType: 'image/png',
      width: editorSourceWidth,
      height: editorSourceHeight,
      fileSizeBytes: 1024,
    }
    return {
      id: '81000000-0000-4000-8000-000000000001',
      sourceAssetId: editorSourceAssetId,
      title: '图片成品',
      schemaVersion: 1,
      version: editorVersion,
      document: editorDocument,
      sourceAsset: asset,
      imageAsset: asset,
      createdAt: '2026-08-05T00:00:00Z',
      updatedAt: '2026-08-05T00:00:00Z',
    }
  }
  return state
}

test('login, studio-only conversation rail, navigation, account menu and theme work', async ({ page }) => {
  const state = await mockApi(page, false)
  await page.goto('/login')
  await expect(page.getByRole('heading', { name: '欢迎回来' })).toBeVisible()
  await page.getByRole('button', { name: '登录' }).click()
  await expect(page).toHaveURL(/\/studio$/)
  await expect(page.getByRole('heading', { name: conversation.title })).toBeVisible()
  await expect(page.getByText('创作会话', { exact: true })).toBeVisible()
  await expect(page.getByLabel('主导航').getByText('任务', { exact: true })).toHaveCount(0)

  await page.getByTitle('历史作品').click()
  await expect(page).toHaveURL(/\/history$/)
  await expect(page.getByRole('heading', { name: '历史作品', exact: true })).toBeVisible()
  await expect(page.locator('.conversation-rail')).toHaveCount(0)

  const themeToggle = page.getByRole('button', { name: '切换主题' })
  await expect(themeToggle).toBeVisible()
  await expect(themeToggle).toContainText('Dark')
  const themeToggleBox = await themeToggle.boundingBox()
  expect(themeToggleBox?.y).toBeLessThan(32)
  expect(page.viewportSize()!.width - themeToggleBox!.x - themeToggleBox!.width).toBeLessThan(40)
  await themeToggle.click()
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark')
  await expect(themeToggle).toContainText('Light')

  await page.getByTitle('账户菜单').click()
  await expect(page.getByText('系统管理员')).toBeVisible()
  await page.getByRole('button', { name: '个人设置' }).click()
  await expect(page.getByRole('heading', { name: '个人设置', exact: true })).toBeVisible()
  await expect(page.getByText('admin', { exact: true })).toBeVisible()
  await page.locator('.dialog-card .n-select').click()
  await page.getByText('跟随系统', { exact: true }).last().click()
  await page.getByRole('button', { name: '保存个人设置' }).click()
  await expect(page.getByText('个人设置已保存')).toBeVisible()
  expect(state.themePreferences).toEqual(['dark', 'system'])

  await page.getByTitle('账户菜单').click()
  await page.keyboard.press('Escape')
  await expect(page.locator('.account-menu')).toHaveCount(0)
  await page.getByTitle('账户菜单').click()
  await page.getByRole('heading', { name: '历史作品', exact: true }).click()
  await expect(page.locator('.account-menu')).toHaveCount(0)

  await page.getByTitle('账户菜单').click()
  await page.getByRole('button', { name: '退出登录' }).click()
  await expect(page.getByText('确认退出当前账号？尚未发送的创作内容不会保存。')).toBeVisible()
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page).toHaveURL(/\/history$/)
  expect(state.logoutRequests).toBe(0)

  await page.getByTitle('账户菜单').click()
  await page.getByRole('button', { name: '退出登录' }).click()
  await page.getByRole('button', { name: '确认退出' }).click()
  await expect(page).toHaveURL(/\/login$/)
  expect(state.logoutRequests).toBe(1)
})

test('new conversations keep the composer visible and studio panels inside the viewport', async ({ page }) => {
  await page.setViewportSize({ width: 2048, height: 960 })
  await mockApi(page, true)
  await page.goto('/studio')

  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', '/favicon.svg')
  const faviconResponse = await page.request.get('/favicon.svg')
  expect(faviconResponse.ok()).toBe(true)
  expect(await faviconResponse.text()).toContain('<svg')

  await page.getByTitle('新建会话').click()
  await expect(page.getByRole('heading', { name: '新会话', exact: true })).toBeVisible()
  await expect(page.locator('.conversation-item.active')).toContainText('新会话')

  const composer = page.locator('.composer')
  await expect(composer).toBeVisible()
  await expect(composer).toBeInViewport()
  await expect(composer.locator('textarea')).toBeEditable()

  const parameterPanel = page.locator('.parameter-panel')
  const themeToggle = page.getByRole('button', { name: '切换主题' })
  const parameterBox = await parameterPanel.boundingBox()
  const themeBox = await themeToggle.boundingBox()
  expect(parameterBox).not.toBeNull()
  expect(themeBox).not.toBeNull()
  expect(themeBox!.x + themeBox!.width).toBeLessThan(parameterBox!.x)
  expect(parameterBox!.x - themeBox!.x - themeBox!.width).toBeLessThan(50)

  const resizeHandle = page.getByRole('separator', { name: '调整生成参数栏宽度' })
  const resizeHandleBox = await resizeHandle.boundingBox()
  expect(resizeHandleBox).not.toBeNull()
  await page.mouse.move(
    resizeHandleBox!.x + resizeHandleBox!.width / 2,
    resizeHandleBox!.y + resizeHandleBox!.height / 2,
  )
  await page.mouse.down()
  await page.mouse.move(resizeHandleBox!.x - 400, resizeHandleBox!.y + resizeHandleBox!.height / 2, { steps: 6 })
  await page.mouse.up()
  await expect.poll(async () => (await parameterPanel.boundingBox())?.width ?? 0).toBeGreaterThan(700)
  await expect(resizeHandle).toHaveAttribute('aria-valuenow', /7\d\d/)
  expect(await parameterPanel.locator('.base-parameters').evaluate((group) => getComputedStyle(group).gridTemplateColumns.split(' ').length)).toBe(4)
  const advancedGrid = parameterPanel.locator('.advanced-parameter-grid')
  expect(await advancedGrid.evaluate((group) => getComputedStyle(group).gridTemplateColumns.split(' ').length)).toBe(3)
  expect((await advancedGrid.boundingBox())!.width).toBeGreaterThan(650)
  const resizedParameterBox = await parameterPanel.boundingBox()
  const resizedThemeBox = await themeToggle.boundingBox()
  expect(resizedParameterBox).not.toBeNull()
  expect(resizedThemeBox).not.toBeNull()
  expect(resizedThemeBox!.x + resizedThemeBox!.width).toBeLessThan(resizedParameterBox!.x)
  expect(resizedParameterBox!.x - resizedThemeBox!.x - resizedThemeBox!.width).toBeLessThan(50)

  const finalParameter = parameterPanel.locator('label').filter({ hasText: '流式局部预览数量' })
  await finalParameter.scrollIntoViewIfNeeded()
  await expect(finalParameter).toBeInViewport()
  await expect(composer).toBeInViewport()
})

test('mobile navigation and studio panels remain reachable without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await mockApi(page, true, { multipleConversations: true, multipleModels: true })
  await page.goto('/studio')

  const noHorizontalOverflow = () => page.evaluate(
    () => document.documentElement.scrollWidth <= window.innerWidth,
  )
  await expect.poll(noHorizontalOverflow).toBe(true)

  await page.getByRole('button', { name: '打开主导航' }).click()
  await expect(page.locator('.main-nav')).toHaveClass(/open/)
  await page.getByRole('link', { name: /Provider/ }).click()
  await expect(page.getByRole('heading', { name: '我的 Provider' })).toBeVisible()
  await expect.poll(noHorizontalOverflow).toBe(true)

  await page.getByRole('button', { name: '打开主导航' }).click()
  await page.getByRole('link', { name: /创作台/ }).click()
  await page.getByRole('button', { name: /会话$/ }).click()
  await expect(page.locator('.conversation-rail')).toHaveClass(/open/)
  await expect.poll(() => page.locator('.conversation-rail').evaluate(
    (panel) => Math.round(panel.getBoundingClientRect().left),
  )).toBe(0)
  await expect(page.locator('.conversation-rail').getByRole('button', { name: '关闭会话列表' })).toBeVisible()
  await page.locator('.conversation-item').filter({ hasText: secondConversation.title }).click()
  await expect(page.locator('.conversation-rail')).not.toHaveClass(/open/)

  await page.getByRole('button', { name: /参数$/ }).click()
  await expect(page.locator('.parameter-panel')).toHaveClass(/open/)
  await expect.poll(() => page.locator('.parameter-panel').evaluate(
    (panel) => Math.round(panel.getBoundingClientRect().right),
  )).toBe(390)
  await expect(page.locator('.parameter-panel').getByText('目标分辨率')).toBeVisible()
  await page.locator('.parameter-panel').getByRole('button', { name: '关闭生成参数' }).click()
  await expect(page.locator('.parameter-panel')).not.toHaveClass(/open/)
  await expect.poll(() => page.locator('.parameter-panel').evaluate(
    (panel) => Math.round(panel.getBoundingClientRect().left),
  )).toBe(390)

  await expect(page.locator('.composer')).toBeInViewport()
  await expect(page.locator('.composer textarea')).toBeEditable()
  await expect.poll(noHorizontalOverflow).toBe(true)

  await page.setViewportSize({ width: 412, height: 915 })
  await expect.poll(noHorizontalOverflow).toBe(true)
})

test('messages show timestamps and live generation elapsed time', async ({ page }) => {
  await mockApi(page, true, { slowTask: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('生成一张带时间信息的测试图片')
  await page.locator('.send-button').click()
  const pendingUser = page.locator('.message-row.user.pending-message')
  const pendingAssistant = page.locator('.message-row.assistant.pending-message')
  await expect(pendingUser).toContainText('生成一张带时间信息的测试图片')
  await expect(pendingUser.locator('.message-time')).toContainText('发送')
  await expect(pendingAssistant.locator('.streaming-line')).toBeVisible()
  await expect(page.locator('.studio-empty')).toHaveCount(0)
  const pendingLayout = await pendingUser.evaluate((row) => {
    const bubble = row.querySelector('.message-body > p')?.getBoundingClientRect()
    const avatar = row.querySelector('.message-avatar')?.getBoundingClientRect()
    const assistant = document
      .querySelector('.message-row.assistant.pending-message .streaming-line')
      ?.getBoundingClientRect()
    return bubble && avatar && assistant
      ? { bubbleX: bubble.x, avatarX: avatar.x, assistantX: assistant.x }
      : null
  })
  expect(pendingLayout).not.toBeNull()
  expect(pendingLayout!.bubbleX).toBeGreaterThan(pendingLayout!.assistantX)
  expect(pendingLayout!.avatarX).toBeGreaterThan(pendingLayout!.bubbleX)
  const taskPill = page.locator('.task-pill')
  await expect(taskPill).toContainText('正在创建任务')
  await expect.poll(() => taskPill.textContent(), { timeout: 3000 }).toMatch(/· [1-9]\d*秒/)

  await expect(page.getByText('已生成 1 张图片')).toBeVisible()
  await expect(page.locator('.message-row.user .message-time')).toContainText('发送')
  await expect(page.locator('.message-row.assistant .message-time')).toContainText('耗时 5秒')
  const generatedImageButton = page.getByRole('button', { name: /放大生成图片/ }).first()
  const generatedImage = generatedImageButton.locator('img')
  await expect(page.locator('.message-row.assistant .message-images a[target="_blank"]')).toHaveCount(0)
  await expect(generatedImageButton).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)')
  await expect(generatedImage).toHaveCSS('object-fit', 'contain')
  expect(await generatedImage.evaluate((image) => getComputedStyle(image).aspectRatio)).toBe('auto')
  await expect.poll(() => generatedImage.evaluate((image) => image.complete && image.naturalHeight > image.naturalWidth)).toBe(true)
  const generatedImageBox = await generatedImage.boundingBox()
  expect(generatedImageBox).not.toBeNull()
  expect(generatedImageBox!.height).toBeGreaterThan(generatedImageBox!.width)
  expect(generatedImageBox!.height).toBeLessThanOrEqual(560)
  expect(generatedImageBox!.width).toBeLessThanOrEqual(520)
  const directEditorLink = page.locator('.message-row.assistant .image-edit-button').first()
  await expect(directEditorLink).toBeVisible()

  await generatedImageButton.click()
  const imageDialog = page.getByRole('dialog', { name: /生成图片/ })
  await expect(imageDialog).toBeVisible()
  await expect(imageDialog.getByRole('button', { name: '关闭图片预览' })).toBeVisible()
  await expect.poll(async () => (await imageDialog.locator('img').boundingBox())?.height ?? 0)
    .toBeGreaterThan(generatedImageBox!.height)
  await imageDialog.getByRole('link', { name: '编辑成品' }).click()
  await expect(page).toHaveURL(/\/editor\/73000000-0000-4000-8000-000000000001\?documentId=81000000/)
  const outputSize = page.locator('.dimension-fields input')
  await outputSize.nth(0).fill('960')
  await expect(outputSize.nth(1)).toHaveValue('1024')
  await outputSize.nth(1).fill('128')
  await expect(outputSize.nth(0)).toHaveValue('960')
  await expect(outputSize.nth(1)).toHaveValue('128')
  await page.getByText('裁剪', { exact: true }).click()
  const originalPixelCrop = page.locator('.four-fields input')
  await originalPixelCrop.nth(3).fill('261')
  await expect(originalPixelCrop.nth(3)).toHaveValue('261')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByText('已保存', { exact: true })).toBeVisible()
  await page.goBack()
  await expect(page).toHaveURL(/\/studio$/)

  await directEditorLink.click()
  await expect(page).toHaveURL(/\/editor\/73000000-0000-4000-8000-000000000001/)
  await page.goBack()

  const studioDownload = page.locator('.message-row.assistant .image-download-button').first()
  await expect(studioDownload).toHaveAttribute('download', 'ai-image-studio-73000000-0000-4000-8000-000000000001.png')
  await expect(studioDownload).not.toHaveAttribute('target', '_blank')
  const studioDownloadStarted = page.waitForEvent('download')
  await studioDownload.click()
  expect((await studioDownloadStarted).suggestedFilename()).toBe('ai-image-studio-73000000-0000-4000-8000-000000000001.png')
  await expect(page).toHaveURL(/\/studio$/)
})

test('each conversation can generate independently without blocking a new conversation', async ({ page }) => {
  const state = await mockApi(page, true, { slowTask: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('第一个会话正在生成的图片')
  await page.locator('.send-button').click()
  await expect(page.locator('.message-row.user.pending-message')).toContainText('第一个会话正在生成的图片')

  await page.getByTitle('新建会话').click()
  await expect(page.getByRole('heading', { name: '新会话', exact: true })).toBeVisible()
  await expect(page.locator('.conversation-item').filter({ hasText: conversation.title })).toContainText('生成中')
  await page.locator('.composer textarea').fill('第二个会话同时生成的图片')
  await expect(page.locator('.send-button')).toBeEnabled()
  await page.locator('.send-button').click()
  await expect(page.locator('.message-row.user.pending-message')).toContainText('第二个会话同时生成的图片')

  await expect.poll(() => state.messageConversationIds).toEqual([
    conversation.id,
    '30000000-0000-4000-8000-000000000099',
  ])
  await expect(page.getByText('已生成 1 张图片', { exact: true })).toBeVisible({ timeout: 8000 })

  await page.locator('.conversation-item').filter({ hasText: conversation.title }).click()
  await expect(page.getByText('第一个会话正在生成的图片', { exact: true })).toBeVisible()
  await expect(page.getByText('已生成 1 张图片', { exact: true })).toBeVisible()
})

test('clipboard images show thumbnails inside the composer and upload with the message', async ({ page }) => {
  const state = await mockApi(page, true)
  await page.goto('/studio')

  const composer = page.locator('.composer textarea')
  await composer.evaluate(async (textarea) => {
    const imageBlob = async (color: string, type: string) => {
      const canvas = document.createElement('canvas')
      canvas.width = 96
      canvas.height = 64
      const context = canvas.getContext('2d')!
      context.fillStyle = color
      context.fillRect(0, 0, canvas.width, canvas.height)
      context.fillStyle = 'rgba(255,255,255,.85)'
      context.fillRect(12, 12, 36, 40)
      return new Promise<Blob>((resolve, reject) => {
        canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('无法创建测试图片')), type)
      })
    }
    const clipboard = new DataTransfer()
    clipboard.items.add(new File([await imageBlob('#7c4dff', 'image/png')], 'clipboard-reference.png', { type: 'image/png' }))
    clipboard.items.add(new File([await imageBlob('#f05a74', 'image/webp')], 'second-reference.webp', { type: 'image/webp' }))
    textarea.dispatchEvent(new ClipboardEvent('paste', {
      bubbles: true,
      cancelable: true,
      clipboardData: clipboard,
    }))
  })

  await expect(page.getByText('已粘贴 2 张参考图')).toBeVisible()
  const previews = page.locator('.composer .composer-attachment img')
  await expect(previews).toHaveCount(2)
  await expect(previews.first()).toHaveAttribute('alt', '参考图 clipboard-reference.png')
  await expect(previews.first()).toHaveAttribute('src', /^blob:/)
  await expect(page.getByText('clipboard-reference.png')).toHaveCount(0)
  await page.getByRole('button', { name: '移除参考图 clipboard-reference.png' }).click()
  await expect(previews).toHaveCount(1)
  await expect(previews.first()).toHaveAttribute('alt', '参考图 second-reference.webp')
  await composer.fill('基于粘贴的参考图生成')
  await page.locator('.send-button').click()

  await expect(page.getByText('已生成 1 张图片')).toBeVisible()
  await expect(previews).toHaveCount(0)
  expect(state.uploadedAssetIds).toHaveLength(1)
  expect(state.messageRequests.at(-1)).toMatchObject({
    inputAssetIds: state.uploadedAssetIds,
  })
})

test('ordinary users are not forced to change password and retain the manual action', async ({ page }) => {
  await mockApi(page, true, {
    authUser: {
      ...user,
      username: 'ordinary-user',
      displayName: '普通用户',
      role: 'user',
      mustChangePassword: true,
    },
  })
  await page.goto('/studio')
  await expect(page.getByRole('heading', { name: conversation.title })).toBeVisible()
  await expect(page.getByRole('heading', { name: '修改密码', exact: true })).toHaveCount(0)
  await expect(page.getByText('默认管理员密码必须修改后才能继续使用其他功能。')).toHaveCount(0)

  await page.getByTitle('账户菜单').click()
  await page.getByRole('button', { name: '修改密码' }).click()
  await expect(page.getByRole('heading', { name: '修改密码', exact: true })).toBeVisible()
  await expect(page.getByText('默认管理员密码必须修改后才能继续使用其他功能。')).toHaveCount(0)
})

test('flagged administrators must change password before dismissing the dialog', async ({ page }) => {
  await mockApi(page, true, { authUser: { ...user, mustChangePassword: true } })
  await page.goto('/studio')
  await expect(page.locator('.dialog-card .n-card-header__main')).toHaveText('修改密码')
  await expect(page.getByText('默认管理员密码必须修改后才能继续使用其他功能。')).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.locator('.dialog-card .n-card-header__main')).toHaveText('修改密码')
})

test('provider image-model filtering, test generation, pricing and storage scan are interactive', async ({ page }) => {
  const state = await mockApi(page, true, { includeTextModel: true })
  await page.goto('/providers')
  await expect(page.getByRole('heading', { name: '我的 Provider' })).toBeVisible()
  await expect(page.getByText('连接正常')).toBeVisible()
  await expect(page.getByText('GPT-5.4', { exact: true })).toHaveCount(0)

  await page.getByRole('button', { name: '添加 Provider' }).click()
  await expect(page.getByText('支持任意 OpenAI Compatible 供应商地址')).toBeVisible()
  await page.getByLabel('配置标识').fill('custom-vendor')
  await page.getByLabel('显示名称').fill('Custom Vendor')
  await page.getByLabel('Base URL').fill('https://images.vendor.example/v1')
  await page.getByRole('button', { name: '保存配置' }).click()
  await expect(page.getByText('Provider 已保存')).toBeVisible()
  expect(state.providerWrites).toHaveLength(1)
  expect(state.providerWrites[0]?.baseUrl).toBe('https://images.vendor.example/v1')

  await page.getByRole('button', { name: '测试连接' }).click()
  await expect(page.locator('.connection-test-dialog .n-card-header__main')).toHaveText('测试生图连接')
  await expect(page.getByLabel('测试提示词')).toHaveValue(/cinematic studio photograph/i)
  await expect(page.locator('.test-parameter-grid label').filter({ hasText: '尺寸' })).toContainText('Auto')
  await page.getByLabel('测试提示词').fill('A purple glass sphere on a white background.')
  await page.getByRole('button', { name: '生成测试图片' }).click()
  await expect(page.getByAltText('GPT Image 1 测试生成结果')).toBeVisible()
  await expect(page.getByText('1024 × 1024 · image/png · 1280 ms')).toBeVisible()
  expect(state.testGenerationRequests).toHaveLength(1)
  expect(state.testGenerationRequests[0]).toMatchObject({
    modelId: model.id,
    prompt: 'A purple glass sphere on a white background.',
    parameters: { n: 1, size: 'auto', quality: 'auto' },
  })
  await page.keyboard.press('Escape')

  await page.getByRole('button', { name: '价格' }).click()
  await expect(page.getByText('配置平台成本估算使用的单张图片价格')).toBeVisible()
  await expect(page.getByRole('button', { name: '保存当前价格' })).toBeDisabled()
  await page.keyboard.press('Escape')

  await page.getByTitle('系统设置').click()
  await expect(page.getByRole('heading', { name: '数据库 / 文件一致性' })).toBeVisible()
  await page.getByRole('button', { name: '立即扫描' }).click()
  await expect(page.getByText('一致性扫描完成')).toBeVisible()
  await expect(page.getByText('最近扫描成功')).toBeVisible()
})

test('ordinary users can view model pricing but cannot modify it', async ({ page }) => {
  await mockApi(page, true, {
    authUser: { ...user, username: 'ordinary-user', displayName: '普通用户', role: 'user' },
  })
  await page.goto('/providers')
  await page.getByRole('button', { name: '价格' }).click()
  await expect(page.getByText('USD 0.040000 / 张')).toBeVisible()
  await expect(page.getByText('模型价格由管理员统一维护，普通用户仅可查看。')).toBeVisible()
  await expect(page.getByRole('button', { name: '保存当前价格' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '删除' })).toHaveCount(0)
})

test('advanced image parameters follow model, format and edit visibility rules', async ({ page }) => {
  await mockApi(page, true)
  await page.goto('/studio')

  const panel = page.locator('.parameter-panel')
  await expect(panel.getByText('流式局部预览数量', { exact: true })).toBeVisible()
  await expect(panel.getByText('输出压缩（JPEG/WebP）', { exact: true })).toHaveCount(0)
  await expect(panel.locator('label').filter({ hasText: '输入图片保真度' })).toHaveCount(0)

  const outputFormat = panel.locator('label').filter({ hasText: '输出格式' })
  await outputFormat.locator('.n-select').click()
  await page.getByText('jpeg', { exact: true }).last().click()
  await expect(panel.getByText('输出压缩（JPEG/WebP）', { exact: true })).toBeVisible()

  await page.locator('input[type="file"]').setInputFiles({
    name: 'reference.png',
    mimeType: 'image/png',
    buffer: Buffer.from('reference'),
  })
  await expect(panel.locator('label').filter({ hasText: '输入图片保真度' })).toBeVisible()
})

test('aspect ratio and target resolution stay linked for custom-size models', async ({ page }) => {
  await mockApi(page, true, { customSizeModel: true })
  await page.goto('/studio')

  const panel = page.locator('.parameter-panel')
  const modelSelect = panel.locator('label').filter({ hasText: '生图模型' }).locator('.n-select')
  await modelSelect.click()
  await page.getByText(gptImage2Model.displayName, { exact: true }).last().click()

  const aspect = panel.locator('.aspect-ratio-select')
  const size = panel.locator('.target-resolution-select')
  await aspect.locator('.n-base-selection').first().click()
  await page.getByText('16:9', { exact: true }).last().click()
  await size.locator('.n-base-selection').first().click()
  await expect(page.getByText('3840x2160', { exact: true }).last()).toBeVisible()
  await expect(page.getByText('2160x3840', { exact: true })).toHaveCount(0)
  await page.getByText('3840x2160', { exact: true }).last().click()

  await aspect.locator('.n-base-selection').first().click()
  await page.locator('.n-base-select-option:visible').filter({ hasText: 'Auto（默认）' }).first().click()
  await expect(size.locator('.n-base-selection-label')).toContainText('Auto')
  await size.locator('.n-base-selection').first().click()
  await page.getByText('2160x3840', { exact: true }).last().click()
  await expect(aspect.locator('.n-base-selection-label')).toContainText('9:16')

  await aspect.locator('.n-base-selection').first().click()
  await page.getByText('16:9', { exact: true }).last().click()
  await expect(size.locator('.n-base-selection-label')).toContainText('Auto')
  await size.locator('.n-base-selection').first().click()
  await expect(page.getByText('3840x2160', { exact: true }).last()).toBeVisible()
  await expect(page.getByText('2160x3840', { exact: true })).toHaveCount(0)

  await page.getByText('自定义尺寸…', { exact: true }).last().click()
  await expect(panel.getByText('自定义尺寸', { exact: true })).toBeVisible()
  await expect(panel.getByText('当前数值不符合模型的边长、像素数或 16 倍数限制。')).toHaveCount(0)
})

test('generation parameters are remembered per model across conversations and reloads', async ({ page }) => {
  await mockApi(page, true, { multipleConversations: true, multipleModels: true })
  await page.goto('/studio')

  const panel = page.locator('.parameter-panel')
  const size = panel.locator('label').filter({ hasText: '分辨率' })
  const outputFormat = panel.locator('label').filter({ hasText: '输出格式' })

  await size.locator('.n-select').click()
  await page.getByText('1024x1024', { exact: true }).last().click()
  await outputFormat.locator('.n-select').click()
  await page.getByText('jpeg', { exact: true }).last().click()

  await page.getByText(secondConversation.title, { exact: true }).click()
  await expect(panel.getByText(geminiModel.displayName, { exact: true })).toBeVisible()
  await size.locator('.n-select').click()
  await page.getByText('4k', { exact: true }).last().click()

  await page.getByText(conversation.title, { exact: true }).click()
  await expect(size.locator('.n-base-selection-label')).toContainText('1024x1024')
  await expect(outputFormat.locator('.n-base-selection-label')).toContainText('jpeg')

  await page.getByText(secondConversation.title, { exact: true }).click()
  await expect(size.locator('.n-base-selection-label')).toContainText('4k')

  await page.reload()
  await expect(page.getByRole('heading', { name: secondConversation.title, exact: true })).toBeVisible()
  await expect(page.locator('.conversation-item').filter({ hasText: secondConversation.title })).toHaveClass(/active/)
  await expect(size.locator('.n-base-selection-label')).toContainText('4k')
})

test('conversation title search and complete history filters are functional', async ({ page }) => {
  const state = await mockApi(page, true, { multipleConversations: true })
  await page.goto('/studio')

  const conversationList = page.locator('.conversation-list')
  await page.getByPlaceholder('搜索会话标题').fill('极简香水')
  await expect(conversationList.getByText(secondConversation.title, { exact: true })).toBeVisible()
  await expect(conversationList.getByText(conversation.title, { exact: true })).toHaveCount(0)
  await page.getByPlaceholder('搜索会话标题').fill('不存在的标题')
  await expect(conversationList.getByText('没有匹配的会话标题。')).toBeVisible()

  await page.getByTitle('历史作品').click()
  const historyCard = page.locator('.history-card').first()
  await expect(historyCard).toBeVisible()
  await expect(historyCard.locator('a[target="_blank"]')).toHaveCount(0)
  await historyCard.getByRole('button', { name: '放大历史作品：历史筛选测试图片' }).click()
  const historyImageDialog = page.getByRole('dialog', { name: '历史筛选测试图片' })
  await expect(historyImageDialog).toBeVisible()
  await expect.poll(async () => (await historyImageDialog.locator('img').boundingBox())?.height ?? 0).toBeGreaterThan(400)
  await historyImageDialog.getByRole('button', { name: '关闭图片预览' }).click()
  await expect(historyImageDialog).toHaveCount(0)

  const historyEditor = historyCard.getByRole('link', { name: '编辑成品' })
  await expect(historyEditor).toBeVisible()
  await historyEditor.click()
  await expect(page).toHaveURL(/\/editor\/73000000-0000-4000-8000-000000000099\?documentId=81000000/)
  await expect(page.getByText('图片成品', { exact: true })).toBeVisible()

  const canvasSize = page.locator('.dimension-fields input')
  await canvasSize.nth(0).fill('1920')
  await expect(canvasSize.nth(1)).toHaveValue('1024')
  await canvasSize.nth(1).fill('1080')
  await page.getByText('裁剪', { exact: true }).click()
  const cropSize = page.locator('.four-fields input')
  await cropSize.nth(3).fill('261')
  await expect(cropSize.nth(3)).toHaveValue('261')
  await page.waitForTimeout(1100)
  await page.reload()
  await page.getByText('裁剪', { exact: true }).click()
  await expect(page.locator('.four-fields input').nth(3)).toHaveValue('261')
  await page.goto('/history')
  await expect(historyCard).toBeVisible()

  const historyDownload = historyCard.getByRole('link', { name: '↓ 下载原图' })
  await expect(historyDownload).toHaveAttribute('download', 'ai-image-studio-73000000-0000-4000-8000-000000000099.png')
  const historyDownloadStarted = page.waitForEvent('download')
  await historyDownload.click()
  expect((await historyDownloadStarted).suggestedFilename()).toBe('ai-image-studio-73000000-0000-4000-8000-000000000099.png')
  await expect(page).toHaveURL(/\/history$/)

  await page.getByLabel('开始日期').fill('2026-07-01')
  await page.getByLabel('结束日期').fill('2026-07-21')
  const sizeInputs = page.locator('.size-filter input')
  await sizeInputs.nth(0).fill('1024')
  await sizeInputs.nth(1).fill('1024')
  await page.getByRole('button', { name: '应用筛选' }).click()

  await expect.poll(() => state.historyQueries.length).toBeGreaterThan(1)
  const filtered = state.historyQueries[state.historyQueries.length - 1]!
  expect(filtered).toEqual(expect.objectContaining({ width: '1024', height: '1024' }))
  expect(filtered.createdFrom).toBeTruthy()
  expect(filtered.createdTo).toBeTruthy()

  await page.getByRole('button', { name: '重置' }).click()
  await expect.poll(() => state.historyQueries[state.historyQueries.length - 1]).toEqual({})
})

test('Konva editor keeps exact source-pixel crops, canvas layout and export formats across reloads', async ({ page }) => {
  const pageErrors: string[] = []
  page.on('pageerror', (error) => pageErrors.push(error.message))
  const state = await mockApi(page, true, { editorSourceWidth: 4096, editorSourceHeight: 4096 })
  await page.goto('/editor/73000000-0000-4000-8000-000000000099?documentId=81000000-0000-4000-8000-000000000001')

  await expect(page.getByText('初始原图 4096 × 4096', { exact: true })).toBeVisible()
  await expect(page.locator('.editor-toolbar')).not.toContainText('AI 扩图')

  await page.getByText('裁剪', { exact: true }).click()
  const crop = page.locator('.four-fields input')
  await crop.nth(2).fill('1024')
  await crop.nth(3).fill('576')
  await expect(crop.nth(2)).toHaveValue('1024')
  await expect(crop.nth(3)).toHaveValue('576')
  await expect(page.getByText('成品 1024 × 576', { exact: true })).toBeVisible()

  await page.getByRole('button', { name: '恢复完整原图' }).click()
  await crop.nth(3).fill('261')
  await expect(crop.nth(2)).toHaveValue('4096')
  await expect(crop.nth(3)).toHaveValue('261')
  await expect(page.getByText('成品 4096 × 261', { exact: true })).toBeVisible()

  await page.getByText('选择', { exact: true }).click()
  const canvas = page.locator('.dimension-fields input')
  await canvas.nth(0).fill('1920')
  await canvas.nth(1).fill('1080')
  await page.getByRole('button', { name: '填满画布', exact: true }).click()
  await expect(page.getByText('成品 1920 × 1080', { exact: true })).toBeVisible()

  await page.getByText('背景', { exact: true }).click()
  await page.getByRole('button', { name: '纯色', exact: true }).click()
  await page.getByText('选择', { exact: true }).click()
  await page.locator('.editor-inspector .n-select').first().click()
  await page.getByText('完整显示（保持比例）', { exact: true }).last().click()

  await page.getByRole('button', { name: '导出成品' }).click()
  await expect.poll(() => state.editorExports.length).toBe(1)

  await page.locator('.export-format').click()
  await page.getByText('JPEG', { exact: true }).last().click()
  await page.getByRole('button', { name: '导出成品' }).click()
  await expect.poll(() => state.editorExports.length).toBe(2)

  await page.locator('.export-format').click()
  await page.getByText('WebP', { exact: true }).last().click()
  await page.getByRole('button', { name: '导出成品' }).click()
  await expect.poll(() => state.editorExports.length).toBe(3)
  expect(state.editorExports).toEqual([
    { width: 1920, height: 1080, format: 'png' },
    { width: 1920, height: 1080, format: 'jpeg' },
    { width: 1920, height: 1080, format: 'webp' },
  ])
  expect(state.editorSaves.every((document) => (
    document.image.assetId === '73000000-0000-4000-8000-000000000099'
  ))).toBe(true)

  await page.reload()
  await expect(page.locator('.dimension-fields input').nth(0)).toHaveValue('1920')
  await expect(page.locator('.dimension-fields input').nth(1)).toHaveValue('1080')
  expect(pageErrors).toEqual([])
})

test('usage records load additional cursor pages without replacing totals', async ({ page }) => {
  await mockApi(page, true)
  await page.goto('/usage')

  await expect(page.getByRole('heading', { name: '用量与成本' })).toBeVisible()
  await expect(page.locator('.metric-card').filter({ hasText: '完成任务' }).getByText('75')).toBeVisible()
  const recent = page.locator('.analytics-section').filter({ hasText: '最近用量记录' })
  await expect(recent.locator('tbody tr')).toHaveCount(50)
  await recent.getByRole('button', { name: '加载更多' }).click()
  await expect(recent.locator('tbody tr')).toHaveCount(75)
  await expect(recent.getByRole('button', { name: '加载更多' })).toHaveCount(0)
})

test('failed task creation cleans new uploads and restores the composer draft', async ({ page }) => {
  const state = await mockApi(page, true, { messageCreationFailure: true })
  await page.goto('/studio')

  await page.locator('input[type="file"]').setInputFiles([
    { name: 'draft-first.png', mimeType: 'image/png', buffer: Buffer.from('first') },
    { name: 'draft-second.png', mimeType: 'image/png', buffer: Buffer.from('second') },
  ])
  await page.locator('.composer textarea').fill('这条请求失败后必须保留')
  await page.locator('.send-button').click()

  await expect(page.getByText('任务参数校验失败')).toBeVisible()
  await expect(page.locator('.composer textarea')).toHaveValue('这条请求失败后必须保留')
  await expect(page.locator('.composer-attachment img[alt="参考图 draft-first.png"]')).toBeVisible()
  await expect(page.locator('.composer-attachment img[alt="参考图 draft-second.png"]')).toBeVisible()
  await expect.poll(() => state.assetDeleteRequests).toEqual(state.uploadedAssetIds)
  await expect(page.locator('.message-row')).toHaveCount(0)
})

test('style template creation, ordered uploads, multi-turn prompts and SSE recovery work', async ({ page }) => {
  const state = await mockApi(page, true, {
    disconnectEventStreamOnce: true,
    multipleModels: true,
  })
  await page.goto('/studio')

  const parameterGroups = page.locator('.parameter-group')
  await parameterGroups.first().locator('.n-select').nth(1).click()
  await page.getByText('Gemini Flash Image', { exact: true }).last().click()
  await expect(page.locator('.parameter-panel').getByText('质量', { exact: true })).toHaveCount(0)
  await parameterGroups.nth(1).locator('.n-select').first().click()
  await expect(page.getByText('21:9', { exact: true })).toBeVisible()
  await page.getByText('21:9', { exact: true }).click()

  const styleField = page.locator('.parameter-field').filter({ hasText: '创作风格' })
  await styleField.locator('.n-select').click()
  await page.getByText('数字互联网大会', { exact: true }).last().click()
  await expect(styleField.getByText(`适用场景：${digitalInternetApplicableScenarios}`)).toBeVisible()
  await styleField.getByRole('button', { name: '管理模板' }).click()
  await expect(page.getByText('管理创作风格模板')).toBeVisible()
  const newTemplateButton = page.getByRole('button', { name: '＋ 新建模板' })
  const systemTemplateButton = page.locator('.template-list button').filter({ hasText: '数字互联网大会' })
  await expect(systemTemplateButton).toHaveClass(/active/)
  await expect(page.locator('.template-editor-heading strong')).toHaveText('查看系统模板')
  await expect(page.getByPlaceholder('例如：复古胶片')).toHaveValue('数字互联网大会')
  await expect(page.getByPlaceholder('例如：复古胶片')).toHaveAttribute('readonly', '')
  await expect(page.getByLabel('适用场景')).toHaveValue(digitalInternetApplicableScenarios)
  await expect(page.getByLabel('适用场景')).toHaveAttribute('readonly', '')
  await expect(page.getByLabel('Prompt 文本')).toHaveValue(digitalInternetStylePrompt)
  await expect(page.getByLabel('Prompt 文本')).toHaveAttribute('readonly', '')
  await expect(page.getByRole('button', { name: /保存修改|创建模板/ })).toHaveCount(0)
  await newTemplateButton.click()
  await expect(page.getByPlaceholder('例如：复古胶片')).toHaveValue('')
  await expect(page.getByLabel('适用场景')).toHaveValue('')
  await expect(page.getByLabel('Prompt 文本')).toHaveValue('')
  await page.getByPlaceholder('例如：复古胶片').fill('复古胶片')
  await page.getByLabel('适用场景').fill('品牌人像、旅行与日常生活方式视觉')
  await page.getByLabel('Prompt 文本').fill('vintage film grain')
  await page.getByRole('button', { name: '创建模板' }).click()
  await expect(page.getByText('模板已保存')).toBeVisible()
  expect(state.templateWrites).toEqual([
    expect.objectContaining({
      templateType: 'style',
      title: '复古胶片',
      applicableScenarios: '品牌人像、旅行与日常生活方式视觉',
      prompt: 'vintage film grain',
    }),
  ])

  await styleField.getByRole('button', { name: '管理模板' }).click()
  await page.locator('.template-list button').filter({ hasText: '复古胶片' }).click()
  await expect(page.locator('.template-editor-heading strong')).toHaveText('编辑我的模板')
  await page.getByLabel('适用场景').fill('品牌人像、旅行、咖啡馆与日常生活方式视觉')
  await page.getByRole('button', { name: '保存修改' }).click()
  expect(state.templateWrites[1]).toEqual(expect.objectContaining({
    applicableScenarios: '品牌人像、旅行、咖啡馆与日常生活方式视觉',
  }))

  const input = page.locator('input[type="file"]')
  await input.setInputFiles([
    { name: 'first.png', mimeType: 'image/png', buffer: Buffer.from('first') },
    { name: 'second.png', mimeType: 'image/png', buffer: Buffer.from('second') },
  ])
  await expect(page.locator('.composer-attachment img[alt="参考图 first.png"]')).toBeVisible()
  await expect(page.locator('.composer-attachment img[alt="参考图 second.png"]')).toBeVisible()
  await page.locator('.composer textarea').fill('根据两张参考图生成海报')
  await page.locator('.send-button').click()
  await expect(page.locator('.task-pill')).toContainText('正在恢复连接')
  await expect(page.getByText('根据两张参考图生成海报')).toBeVisible()
  await expect.poll(() => state.resumeEventIds.length).toBe(2)
  expect(state.resumeEventIds).toEqual(['10', '10'])
  expect(state.messageRequests[0]).toEqual(
    expect.objectContaining({
      content: '根据两张参考图生成海报',
      parentMessageId: null,
      inputAssetIds: state.uploadedAssetIds,
      stream: true,
    }),
  )
  expect(state.messageRequests[0]?.parameters).toEqual(
    expect.objectContaining({ aspect_ratio: '21:9', size: 'auto', n: 1 }),
  )
  expect(state.messageRequests[0]?.parameters).not.toHaveProperty('quality')

  await page.locator('.composer textarea').fill('保持主体，把背景改成雨夜')
  await page.locator('.send-button').click()
  await expect(page.getByText('保持主体，把背景改成雨夜')).toBeVisible()
  expect(state.messageRequests[1]).toEqual(
    expect.objectContaining({
      content: '保持主体，把背景改成雨夜',
      parentMessageId: '72000000-0000-4000-8000-000000000001',
      inputAssetIds: [],
      stream: true,
    }),
  )
})

test('message branches can continue, switch and regenerate without flattening history', async ({ page }) => {
  const state = await mockApi(page, true)
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('建立第一版画面')
  await page.locator('.send-button').click()
  await expect(page.getByText('建立第一版画面', { exact: true })).toBeVisible()

  await page.locator('.composer textarea').fill('沿当前方向制作第二轮')
  await page.locator('.send-button').click()
  await expect(page.getByText('沿当前方向制作第二轮', { exact: true })).toBeVisible()
  expect(state.messageRequests[1]).toEqual(expect.objectContaining({
    parentMessageId: '72000000-0000-4000-8000-000000000001',
  }))

  await page.locator('.message-row.assistant').first().getByRole('button', { name: '从这里继续' }).click()
  await expect(page.locator('.branch-anchor')).toContainText('从这里继续')
  await page.locator('.composer textarea').fill('从第一版创建另一条路线')
  await page.locator('.send-button').click()
  await expect.poll(() => state.messageRequests.length).toBe(3)
  expect(state.messageRequests[2]).toEqual(expect.objectContaining({
    content: '从第一版创建另一条路线',
    parentMessageId: '72000000-0000-4000-8000-000000000001',
  }))
  await expect(page.getByText('从第一版创建另一条路线', { exact: true })).toBeVisible()
  await expect(page.getByText('沿当前方向制作第二轮', { exact: true })).toHaveCount(0)

  const activeBranchMessage = page.locator('.message-row.user').filter({ hasText: '从第一版创建另一条路线' })
  await expect(activeBranchMessage.getByText('分支 2 / 2')).toBeVisible()
  await activeBranchMessage.getByRole('button', { name: '上一个分支' }).click()
  await expect(page.getByText('沿当前方向制作第二轮', { exact: true })).toBeVisible()
  await expect(page.getByText('从第一版创建另一条路线', { exact: true })).toHaveCount(0)

  await page.locator('.composer textarea').fill('尚未发送的草稿')
  await page.locator('.message-row.assistant').last().getByRole('button', { name: '重新生成' }).click()
  await expect.poll(() => state.messageRequests.length).toBe(4)
  expect(state.messageRequests[3]).toEqual(expect.objectContaining({
    content: '沿当前方向制作第二轮',
    parentMessageId: '72000000-0000-4000-8000-000000000001',
  }))
  await expect(page.locator('.composer textarea')).toHaveValue('尚未发送的草稿')
  await expect(page.getByText('分支 3 / 3')).toBeVisible()
})

test('historical user messages hide the image automatically carried from the previous generation', async ({ page }) => {
  await mockApi(page, true, { automaticReferenceInHistory: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('建立第一版画面')
  await page.locator('.send-button').click()
  await expect(page.getByText('建立第一版画面', { exact: true })).toBeVisible()

  await page.locator('.composer textarea').fill('保持主体，把背景改成雨夜')
  await page.locator('.send-button').click()

  const followUp = page.locator('.message-row.user').filter({ hasText: '保持主体，把背景改成雨夜' })
  await expect(followUp).toBeVisible()
  await expect(followUp.locator('.message-images')).toHaveCount(0)
  await expect(page.locator('.message-row.assistant .message-images')).toHaveCount(2)
})

test('a model rejection explains the reason in the failed assistant message', async ({ page }) => {
  await mockApi(page, true, { rejectedTask: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('生成一张会被模型拒绝的图片')
  await page.locator('.send-button').click()

  await expect(page.locator('.task-error')).toContainText('图片生成请求未通过安全检查')
  await expect(page.locator('.task-error')).toContainText('调整提示词或输入图片后重试')
})

test('a failed generation keeps its error summary and can retry the same task', async ({ page }) => {
  const state = await mockApi(page, true, { retryableTask: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('生成一张会先失败再重试的图片')
  await page.locator('.send-button').click()

  await expect(page.locator('.task-error')).toContainText('上游服务暂时不可用')
  await expect(page.locator('.message-row.assistant .message-time')).toContainText('耗时 1分5秒')
  await page.getByRole('button', { name: '重试', exact: true }).click()

  await expect.poll(() => state.retryRequests).toEqual([
    '70000000-0000-4000-8000-000000000001',
  ])
  await expect.poll(() => state.resumeEventIds).toContain('500')
  await expect(page.getByText('已生成 1 张图片', { exact: true })).toBeVisible()
  await expect(page.locator('.task-error')).toHaveCount(0)
})

test('an active generation can be cancelled and reloads as a terminal message', async ({ page }) => {
  const state = await mockApi(page, true, { cancellableTask: true })
  await page.goto('/studio')

  await page.locator('.composer textarea').fill('生成一张稍后取消的图片')
  await page.locator('.send-button').click()
  await page.getByRole('button', { name: '取消生成' }).click()

  await expect.poll(() => state.cancelRequests).toEqual([
    '70000000-0000-4000-8000-000000000001',
  ])
  await expect(page.getByText('生成已取消', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '取消生成' })).toHaveCount(0)
  await expect(page.locator('.message-row.assistant .streaming-line')).toHaveCount(0)
})

test('HTML prototype exposes the same message branch interactions', async ({ page }) => {
  const prototypeUrl = pathToFileURL(resolve(process.cwd(), '../docs/ui-prototype.html')).href
  await page.goto(prototypeUrl)

  await page.getByRole('button', { name: '打开账户菜单' }).click()
  await page.getByRole('menuitem', { name: /个人设置/ }).click()
  await expect(page.locator('#accountSettingsModal')).toBeVisible()
  await page.locator('#prototypeThemePreference').selectOption('light')
  await page.getByRole('button', { name: '保存个人设置' }).click()
  await expect(page.locator('body')).toHaveAttribute('data-theme', 'light')

  await page.getByLabel('搜索会话').fill('极简香水')
  await expect(page.locator('.session:visible')).toHaveCount(1)
  await page.getByLabel('搜索会话').fill('')
  await page.locator('[data-view="history"]').click()
  await page.locator('#prototypeHistoryProvider').selectOption('gemini')
  await page.locator('#prototypeHistorySize').selectOption('1024x1024')
  await page.getByRole('button', { name: '应用筛选' }).click()
  await expect(page.locator('[data-history-card]:visible')).toHaveCount(1)
  await page.locator('[data-view="studio"]').click()

  await expect(page.getByText('分支 2 / 2')).toBeVisible()
  await page.getByRole('button', { name: '上一个分支' }).click()
  await expect(page.getByText('分支 1 / 2')).toBeVisible()
  await expect(page.getByText('保持暖色夕阳和留白，只把人物服装改为黑色风衣。')).toBeVisible()
  await page.getByRole('button', { name: '下一个分支' }).click()

  await page.getByRole('button', { name: '从这里继续' }).click()
  await expect(page.locator('#prototypeBranchAnchor')).toBeVisible()
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await expect(page.locator('#prototypeBranchAnchor')).toBeHidden()

  await page.getByRole('button', { name: '重新生成' }).click()
  await expect(page.locator('#toast')).toContainText('复用原用户消息与输入图片创建同级分支')

  await page.getByRole('button', { name: '重试', exact: true }).click()
  await expect(page.locator('#prototypeRetryStatus')).toHaveText('正在重新生成')
  await expect(page.locator('#prototypeRetryDetail')).toContainText('新的 SSE 事件游标')

  await page.locator('#prompt').fill('模拟一条需要恢复连接的任务')
  await page.getByRole('button', { name: '发送' }).click()
  await expect(page.locator('#prototypeTaskStatus')).toHaveText('正在恢复连接')
  await expect(page.locator('#prototypeTaskDetail')).toContainText('Last-Event-ID')

  await page.getByRole('button', { name: '取消生成' }).click()
  await expect(page.locator('#prototypeTaskStatus')).toHaveText('生成已取消')
  await expect(page.locator('#prototypeTaskProgress')).toBeHidden()
})
