import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseEvent, streamPost, streamTask } from './client'

afterEach(() => {
  vi.unstubAllGlobals()
})

function sseResponse(body: string): Response {
  return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } })
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
