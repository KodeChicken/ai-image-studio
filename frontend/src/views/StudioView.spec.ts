// @vitest-environment jsdom

import { flushPromises, shallowMount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import StudioView from './StudioView.vue'

const apiMock = vi.hoisted(() => vi.fn())
const streamPostMock = vi.hoisted(() => vi.fn())
const dialogWarningMock = vi.hoisted(() => vi.fn())

vi.mock('@/api/client', () => ({
  api: apiMock,
  streamPost: streamPostMock,
  streamTask: vi.fn(),
}))

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({ user: { id: 'user-1' } }),
}))

vi.mock('naive-ui', async () => {
  const actual = await vi.importActual<typeof import('naive-ui')>('naive-ui')
  return {
    ...actual,
    useMessage: () => ({
      error: vi.fn(),
      success: vi.fn(),
      warning: vi.fn(),
    }),
    useDialog: () => ({ warning: dialogWarningMock }),
  }
})

const timestamp = '2026-07-30T09:00:00.000Z'
const conversation = {
  id: 'conversation-1',
  title: '测试会话',
  status: 'active',
  defaultProviderId: 'provider-1',
  defaultModelId: 'model-1',
  sortOrder: 0,
  lastMessageAt: timestamp,
  createdAt: timestamp,
  updatedAt: timestamp,
}

describe('StudioView', () => {
  let completeStream: (() => void) | undefined
  let emitStreamEvent: ((event: { type: string; data: Record<string, unknown> }) => void) | undefined
  let holdConversationListReload: boolean
  let releaseConversationListReload: (() => void) | undefined
  let conversationResponse: Array<typeof conversation>
  let conversationMessages: Array<Record<string, unknown>>

  beforeEach(() => {
    apiMock.mockReset()
    streamPostMock.mockReset()
    dialogWarningMock.mockReset()
    completeStream = undefined
    emitStreamEvent = undefined
    holdConversationListReload = false
    releaseConversationListReload = undefined
    conversationResponse = [conversation]
    conversationMessages = []
    localStorage.clear()
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', {
      configurable: true,
      value: vi.fn(),
    })
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn(() => 'blob:reference-preview'),
      revokeObjectURL: vi.fn(),
    })
    apiMock.mockImplementation(async (path: string, init: RequestInit = {}) => {
      if (path === '/api/v1/conversations') {
        if (holdConversationListReload) {
          return new Promise<Array<typeof conversation>>((resolve) => {
            releaseConversationListReload = () => resolve(conversationResponse)
          })
        }
        return conversationResponse
      }
      if (path.startsWith('/api/v1/conversations/') && init.method === 'DELETE') return undefined
      if (path.startsWith('/api/v1/conversations/')) {
        const id = path.slice('/api/v1/conversations/'.length)
        const selected = conversationResponse.find((item) => item.id === id)
        if (selected) return { ...selected, messages: conversationMessages }
      }
      if (path === '/api/v1/providers') {
        return [{
          id: 'provider-1',
          providerKey: 'openai',
          providerType: 'openai-compatible',
          displayName: 'OpenAI Compatible',
          baseUrl: 'https://example.test/v1',
          enabled: true,
          configJson: {},
          credentialConfigured: true,
          modelCount: 1,
          healthStatus: 'healthy',
          lastHealthCheckedAt: timestamp,
          lastHealthError: null,
          createdAt: timestamp,
          updatedAt: timestamp,
        }]
      }
      if (path === '/api/v1/models?includeDiscovered=true') {
        return [{
          id: 'model-1',
          providerId: 'provider-1',
          providerType: 'openai-compatible',
          modelKey: 'gpt-image-2',
          upstreamModelId: 'gpt-image-2',
          displayName: 'GPT Image 2',
          capabilities: {},
          parameterSchema: { parameters: {} },
          availabilityStatus: 'verified',
          discoverySource: 'test',
          capabilitySource: 'test',
          lastDiscoveredAt: timestamp,
          lastVerifiedAt: timestamp,
          enabled: true,
        }]
      }
      if (path === '/api/v1/prompt-templates?templateType=style') return []
      if (path === '/api/v1/image-assets/uploads') {
        return {
          id: 'asset-1',
          contentUrl: '/api/v1/image-assets/asset-1/content',
          mimeType: 'image/png',
          width: 100,
          height: 100,
          fileSizeBytes: 4,
        }
      }
      throw new Error(`Unexpected API request: ${path}`)
    })
    streamPostMock.mockImplementation((
      _path: string,
      _body: Record<string, unknown>,
      onEvent: (event: { type: string; data: Record<string, unknown> }) => void,
    ) => new Promise<void>((resolve) => {
      completeStream = resolve
      emitStreamEvent = onEvent
    }))
  })

  afterEach(() => {
    completeStream?.()
    vi.unstubAllGlobals()
  })

  it('keeps the selected reference image visible while generation is running', async () => {
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    const file = new File(['test'], 'reference.png', { type: 'image/png' })
    const input = wrapper.get<HTMLInputElement>('input[type="file"]')
    Object.defineProperty(input.element, 'files', { configurable: true, value: [file] })
    await input.trigger('change')
    await wrapper.get('textarea').setValue('参考这张图生成横版海报')

    await wrapper.get('.send-button').trigger('click')
    await flushPromises()

    const pendingImage = wrapper.get('.pending-message img[alt="参考图 reference.png"]')
    expect(pendingImage.attributes('src')).toBe('blob:reference-preview')
    expect(wrapper.get('.pending-message .message-image-meta').text()).toContain('reference.png')

    completeStream?.()
    await flushPromises()
    wrapper.unmount()
  })

  it('shows the most recently used conversation first', async () => {
    conversationResponse = [
      conversation,
      {
        ...conversation,
        id: 'conversation-2',
        title: '最近会话',
        lastMessageAt: '2026-07-31T09:00:00.000Z',
        createdAt: '2026-07-31T08:00:00.000Z',
        updatedAt: '2026-07-31T09:00:00.000Z',
      },
    ]
    const wrapper = shallowMount(StudioView)
    await flushPromises()

    expect(wrapper.findAll('.conversation-item strong').map((item) => item.text())).toEqual([
      '最近会话',
      '测试会话',
    ])

    wrapper.unmount()
  })

  it('reserves generated image space and keeps the timeline at the bottom after loading', async () => {
    conversationMessages = [{
      id: 'assistant-message-1',
      conversationId: conversation.id,
      parentMessageId: null,
      role: 'assistant',
      status: 'completed',
      sequenceNo: 1,
      content: '生成完成',
      metadata: {},
      taskId: null,
      taskErrorCode: null,
      taskErrorMessage: null,
      taskRetryCount: null,
      taskStartedAt: null,
      taskFinishedAt: timestamp,
      assets: [{
        id: 'asset-4k',
        contentUrl: '/api/v1/image-assets/asset-4k/content',
        mimeType: 'image/png',
        width: 3840,
        height: 2160,
        fileSizeBytes: 10_000_000,
        relationType: 'generated',
      }],
      createdAt: timestamp,
      updatedAt: timestamp,
    }]
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    const image = wrapper.get('img[alt="生成图片 asset-4k"]')

    expect(image.attributes('width')).toBe('3840')
    expect(image.attributes('height')).toBe('2160')
    const scrollTo = vi.mocked(HTMLElement.prototype.scrollTo)
    scrollTo.mockClear()
    await image.trigger('load')
    await flushPromises()

    expect(scrollTo).toHaveBeenCalledWith({ top: 0, behavior: 'auto' })
    wrapper.unmount()
  })

  it('deletes the active conversation after confirmation and selects the next one', async () => {
    conversationResponse = [
      conversation,
      {
        ...conversation,
        id: 'conversation-2',
        title: '保留会话',
        lastMessageAt: '2026-07-29T09:00:00.000Z',
        createdAt: '2026-07-29T08:00:00.000Z',
        updatedAt: '2026-07-29T09:00:00.000Z',
      },
    ]
    const wrapper = shallowMount(StudioView)
    await flushPromises()

    await wrapper.get('button[aria-label="删除会话 测试会话"]').trigger('click')
    expect(dialogWarningMock).toHaveBeenCalledTimes(1)
    const confirmation = dialogWarningMock.mock.calls[0]![0] as {
      onPositiveClick: () => Promise<void>
    }
    await confirmation.onPositiveClick()
    await flushPromises()

    expect(apiMock).toHaveBeenCalledWith('/api/v1/conversations/conversation-1', { method: 'DELETE' })
    expect(wrapper.findAll('.conversation-item strong').map((item) => item.text())).toEqual(['保留会话'])
    expect(wrapper.get('.studio-header h1').text()).toBe('保留会话')
    wrapper.unmount()
  })

  it('shows the first prompt title as soon as the task is created', async () => {
    conversationResponse = [{ ...conversation, title: '新会话' }]
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    await wrapper.get('textarea').setValue('生成一张夏日海边宣传海报')
    await wrapper.get('.send-button').trigger('click')
    await flushPromises()
    holdConversationListReload = true

    emitStreamEvent?.({
      type: 'task.created',
      data: {
        taskId: 'task-1',
        conversationTitle: '生成一张夏日海边宣传海报',
      },
    })
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.conversation-item strong').text()).toBe('生成一张夏日海边宣传海报')
    conversationResponse = [{ ...conversation, title: '生成一张夏日海边宣传海报' }]
    holdConversationListReload = false
    releaseConversationListReload?.()
    completeStream?.()
    await flushPromises()
    wrapper.unmount()
  })

  it('sends the prompt when Enter is pressed', async () => {
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    const textarea = wrapper.get('textarea')
    await textarea.setValue('生成一张横版海报')
    const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })

    textarea.element.dispatchEvent(event)
    await flushPromises()

    expect(event.defaultPrevented).toBe(true)
    expect(streamPostMock).toHaveBeenCalledTimes(1)

    completeStream?.()
    await flushPromises()
    wrapper.unmount()
  })

  it('keeps Shift+Enter available for line breaks', async () => {
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    const textarea = wrapper.get('textarea')
    await textarea.setValue('第一行')
    const event = new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true })

    textarea.element.dispatchEvent(event)

    expect(event.defaultPrevented).toBe(false)
    expect(streamPostMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('does not send while an input method composition is active', async () => {
    const wrapper = shallowMount(StudioView)
    await flushPromises()
    const textarea = wrapper.get('textarea')
    await textarea.setValue('中文输入')

    await textarea.trigger('keydown', { key: 'Enter', isComposing: true })

    expect(streamPostMock).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})
