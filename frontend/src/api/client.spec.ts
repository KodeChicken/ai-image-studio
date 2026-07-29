import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseEvent, streamPost, streamTask } from './client'

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

function sseResponse(body: string): Response {
  return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } })
}

function controlledSseResponse(initialBody: string) {
  const encoder = new TextEncoder()
  const cancelled = vi.fn()
  let controller: ReadableStreamDefaultController<Uint8Array>
  const response = new Response(new ReadableStream<Uint8Array>({
    start(value) {
      controller = value
      controller.enqueue(encoder.encode(initialBody))
    },
    cancel: cancelled,
  }), { status: 200, headers: { 'Content-Type': 'text/event-stream' } })
  return {
    response,
    cancelled,
    enqueue: (body: string) => controller.enqueue(encoder.encode(body)),
    close: () => controller.close(),
  }
}

describe('parseEvent', () => {
  it('parses a persisted task event', () => {
    expect(
      parseEvent('id: 42\nevent: task.progress\ndata: {"stage":"provider.processing"}'),
    ).toEqual({
      id: '42',
      type: 'task.progress',
      data: { stage: 'provider.processing' },
    })
  })

  it('ignores SSE heartbeat comments', () => {
    expect(parseEvent(': heartbeat')).toBeNull()
  })

  it('parses CRLF-delimited events', () => {
    expect(parseEvent('id: 7\r\nevent: task.completed\r\ndata: {"taskId":"task-7"}')).toEqual({
      id: '7',
      type: 'task.completed',
      data: { taskId: 'task-7' },
    })
  })
})

describe('streamPost', () => {
  it('resumes a disconnected stream with Last-Event-ID without duplicating events', async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        sseResponse('id: 10\nevent: task.created\ndata: {"taskId":"task-1"}\n\n'),
      )
      .mockResolvedValueOnce(
        sseResponse(
          'id: 11\r\nevent: task.progress\r\ndata: {"taskId":"task-1","stage":"provider.processing"}\r\n\r\nid: 12\r\nevent: task.completed\r\ndata: {"taskId":"task-1"}\r\n\r\n',
        ),
      )
    vi.stubGlobal('fetch', fetchMock)
    const events: string[] = []

    await streamPost('/api/v1/conversations/c1/messages', { content: 'test' }, (event) => {
      events.push(`${event.id}:${event.type}`)
    }, { reconnectDelayMs: 0 })

    expect(events).toEqual([
      '10:task.created',
      ':stream.reconnecting',
      '11:task.progress',
      '12:task.completed',
    ])
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      '/api/v1/tasks/task-1/events',
      expect.objectContaining({ headers: { Accept: 'text/event-stream', 'Last-Event-ID': '10' } }),
    )
  })

  it('falls back to task polling when SSE recovery keeps failing', async () => {
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        sseResponse('id: 20\nevent: task.created\ndata: {"taskId":"task-2"}\n\n'),
      )
      .mockRejectedValueOnce(new TypeError('connection reset'))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ status: 'succeeded' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      )
    vi.stubGlobal('fetch', fetchMock)
    const events: string[] = []

    await streamPost('/api/v1/conversations/c1/messages', { content: 'test' }, (event) => {
      events.push(event.type)
    }, { reconnectAttempts: 1, reconnectDelayMs: 0, pollAttempts: 1, pollIntervalMs: 0 })

    expect(events).toEqual([
      'task.created',
      'stream.reconnecting',
      'stream.polling',
      'task.completed',
    ])
    expect(fetchMock).toHaveBeenNthCalledWith(
      3,
      '/api/v1/tasks/task-2',
      expect.objectContaining({ credentials: 'include' }),
    )
  })

  it('actively reconnects a task stream that stops delivering data', async () => {
    vi.useFakeTimers()
    const stalled = controlledSseResponse(
      'id: 30\nevent: task.created\ndata: {"taskId":"task-3"}\n\n',
    )
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(stalled.response)
      .mockResolvedValueOnce(
        sseResponse('id: 31\nevent: task.completed\ndata: {"taskId":"task-3"}\n\n'),
      )
    vi.stubGlobal('fetch', fetchMock)
    const events: string[] = []

    const completed = streamPost('/api/v1/conversations/c1/messages', { content: 'test' }, (event) => {
      events.push(event.type)
    }, { inactivityTimeoutMs: 45_000, reconnectDelayMs: 0 })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(45_000)
    await completed

    expect(stalled.cancelled).toHaveBeenCalledOnce()
    expect(events).toEqual(['task.created', 'stream.reconnecting', 'task.completed'])
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      '/api/v1/tasks/task-3/events',
      expect.objectContaining({ headers: { Accept: 'text/event-stream', 'Last-Event-ID': '30' } }),
    )
  })

  it('treats heartbeat data as activity and keeps a healthy stream connected', async () => {
    vi.useFakeTimers()
    const active = controlledSseResponse(
      'id: 40\nevent: task.created\ndata: {"taskId":"task-4"}\n\n',
    )
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(active.response)
    vi.stubGlobal('fetch', fetchMock)
    const events: string[] = []

    const completed = streamPost('/api/v1/conversations/c1/messages', { content: 'test' }, (event) => {
      events.push(event.type)
    }, { inactivityTimeoutMs: 45_000, reconnectDelayMs: 0 })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(30_000)
    active.enqueue(': heartbeat\n\n')
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(30_000)
    active.enqueue('id: 41\nevent: task.completed\ndata: {"taskId":"task-4"}\n\n')
    active.close()
    await completed

    expect(fetchMock).toHaveBeenCalledOnce()
    expect(active.cancelled).not.toHaveBeenCalled()
    expect(events).toEqual(['task.created', 'task.completed'])
  })
})

describe('streamTask', () => {
  it('resumes a manually retried task after the retry transition event', async () => {
    const fetchMock = vi.fn<typeof fetch>().mockResolvedValueOnce(
      sseResponse(
        'id: 42\nevent: task.progress\ndata: {"taskId":"task-3","stage":"provider.processing"}\n\nid: 43\nevent: task.completed\ndata: {"taskId":"task-3"}\n\n',
      ),
    )
    vi.stubGlobal('fetch', fetchMock)
    const events: string[] = []

    await streamTask('task-3', (event) => events.push(event.type), {
      initialLastEventId: '41',
      reconnectDelayMs: 0,
    })

    expect(events).toEqual(['task.progress', 'task.completed'])
    expect(fetchMock).toHaveBeenCalledWith(
      '/api/v1/tasks/task-3/events',
      expect.objectContaining({ headers: { Accept: 'text/event-stream', 'Last-Event-ID': '41' } }),
    )
  })
})
