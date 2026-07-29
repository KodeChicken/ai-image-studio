export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code?: string,
  ) {
    super(message)
  }
}

interface StreamEvent {
  id: string
  type: string
  data: Record<string, unknown>
}

interface StreamPostOptions {
  reconnectAttempts?: number
  reconnectDelayMs?: number
  pollAttempts?: number
  pollIntervalMs?: number
  initialLastEventId?: string
  inactivityTimeoutMs?: number
}

const terminalEvents = new Set(['task.completed', 'task.failed', 'task.cancelled'])

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body && !(init.body instanceof FormData)) headers.set('Content-Type', 'application/json')
  const response = await fetch(path, { ...init, headers, credentials: 'include' })
  if (!response.ok) {
    const payload = (await response.json().catch(() => null)) as
      | { error?: { message?: string; code?: string } }
      | null
    throw new ApiError(payload?.error?.message ?? `请求失败（${response.status}）`, response.status, payload?.error?.code)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export async function streamPost(
  path: string,
  body: unknown,
  onEvent: (event: StreamEvent) => void,
  options: StreamPostOptions = {},
): Promise<void> {
  const reconnectAttempts = options.reconnectAttempts ?? 3
  const reconnectDelayMs = options.reconnectDelayMs ?? 500
  const pollAttempts = options.pollAttempts ?? 300
  const pollIntervalMs = options.pollIntervalMs ?? 1000
  const inactivityTimeoutMs = Math.max(0, options.inactivityTimeoutMs ?? 45_000)
  let taskId = ''
  let lastEventId = ''
  let terminal = false

  const acceptEvent = (event: StreamEvent) => {
    if (event.id) lastEventId = event.id
    const eventTaskId = event.data.taskId
    if (typeof eventTaskId === 'string') taskId = eventTaskId
    terminal = terminalEvents.has(event.type)
    onEvent(event)
  }

  const initial = await fetch(path, {
    method: 'POST',
    credentials: 'include',
    headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream' },
    body: JSON.stringify(body),
  })
  await assertEventStream(initial, '生成请求失败')
  try {
    await consumeEventStream(initial, acceptEvent, inactivityTimeoutMs)
  } catch (error) {
    if (!taskId) throw error
  }
  if (terminal) return
  if (!taskId) throw new Error('生成事件流已中断，且未收到可恢复的任务 ID')

  for (let attempt = 0; attempt < reconnectAttempts && !terminal; attempt += 1) {
    onEvent({
      id: '',
      type: 'stream.reconnecting',
      data: { taskId, attempt: attempt + 1, maxAttempts: reconnectAttempts },
    })
    if (reconnectDelayMs > 0) await delay(reconnectDelayMs * (attempt + 1))
    try {
      const headers: Record<string, string> = { Accept: 'text/event-stream' }
      if (lastEventId) headers['Last-Event-ID'] = lastEventId
      const resumed = await fetch(`/api/v1/tasks/${taskId}/events`, {
        credentials: 'include',
        headers,
      })
      await assertEventStream(resumed, '恢复生成进度失败')
      await consumeEventStream(resumed, acceptEvent, inactivityTimeoutMs)
    } catch (error) {
      if (error instanceof ApiError && [401, 403, 404].includes(error.status)) throw error
    }
  }

  if (terminal) return
  onEvent({ id: '', type: 'stream.polling', data: { taskId } })
  await pollTaskUntilTerminal(taskId, onEvent, pollAttempts, pollIntervalMs)
}

export async function streamTask(
  taskId: string,
  onEvent: (event: StreamEvent) => void,
  options: StreamPostOptions = {},
): Promise<void> {
  const reconnectAttempts = options.reconnectAttempts ?? 3
  const reconnectDelayMs = options.reconnectDelayMs ?? 500
  const pollAttempts = options.pollAttempts ?? 300
  const pollIntervalMs = options.pollIntervalMs ?? 1000
  const inactivityTimeoutMs = Math.max(0, options.inactivityTimeoutMs ?? 45_000)
  let lastEventId = options.initialLastEventId ?? ''
  let terminal = false

  const acceptEvent = (event: StreamEvent) => {
    if (event.id) lastEventId = event.id
    terminal = terminalEvents.has(event.type)
    onEvent(event)
  }

  for (let attempt = 0; attempt < Math.max(1, reconnectAttempts) && !terminal; attempt += 1) {
    if (attempt > 0) {
      onEvent({
        id: '',
        type: 'stream.reconnecting',
        data: { taskId, attempt, maxAttempts: reconnectAttempts },
      })
      if (reconnectDelayMs > 0) await delay(reconnectDelayMs * attempt)
    }
    try {
      const headers: Record<string, string> = { Accept: 'text/event-stream' }
      if (lastEventId) headers['Last-Event-ID'] = lastEventId
      const response = await fetch(`/api/v1/tasks/${taskId}/events`, {
        credentials: 'include',
        headers,
      })
      await assertEventStream(response, '恢复重试进度失败')
      await consumeEventStream(response, acceptEvent, inactivityTimeoutMs)
    } catch (error) {
      if (error instanceof ApiError && [401, 403, 404].includes(error.status)) throw error
    }
  }

  if (terminal) return
  onEvent({ id: '', type: 'stream.polling', data: { taskId } })
  await pollTaskUntilTerminal(taskId, onEvent, pollAttempts, pollIntervalMs)
}

async function assertEventStream(response: Response, fallbackMessage: string): Promise<void> {
  if (response.ok && response.body) return
  const payload = (await response.json().catch(() => null)) as
    | { error?: { message?: string; code?: string } }
    | null
  throw new ApiError(
    payload?.error?.message ?? fallbackMessage,
    response.status,
    payload?.error?.code,
  )
}

async function consumeEventStream(
  response: Response,
  onEvent: (event: StreamEvent) => void,
  inactivityTimeoutMs: number,
): Promise<void> {
  if (!response.body) throw new Error('事件流响应没有可读取的内容')
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (true) {
      const { value, done } = await readStreamChunk(reader, inactivityTimeoutMs)
      buffer += decoder.decode(value, { stream: !done })
      let boundary = eventBoundary(buffer)
      while (boundary) {
        const block = buffer.slice(0, boundary.index)
        buffer = buffer.slice(boundary.index + boundary.length)
        const parsed = parseEvent(block)
        if (parsed) onEvent(parsed)
        boundary = eventBoundary(buffer)
      }
      if (done) {
        const parsed = parseEvent(buffer.trim())
        if (parsed) onEvent(parsed)
        return
      }
    }
  } finally {
    reader.releaseLock()
  }
}

async function readStreamChunk(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  inactivityTimeoutMs: number,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  if (inactivityTimeoutMs === 0) return reader.read()
  let timeout: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      reader.read(),
      new Promise<never>((_, reject) => {
        timeout = globalThis.setTimeout(
          () => reject(new Error(`生成事件流连续 ${Math.ceil(inactivityTimeoutMs / 1000)} 秒没有数据`)),
          inactivityTimeoutMs,
        )
      }),
    ])
  } catch (error) {
    await reader.cancel().catch(() => undefined)
    throw error
  } finally {
    if (timeout !== undefined) globalThis.clearTimeout(timeout)
  }
}

function eventBoundary(buffer: string): { index: number; length: number } | null {
  const match = /\r?\n\r?\n/.exec(buffer)
  return match ? { index: match.index, length: match[0].length } : null
}

async function pollTaskUntilTerminal(
  taskId: string,
  onEvent: (event: StreamEvent) => void,
  attempts: number,
  intervalMs: number,
): Promise<void> {
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (intervalMs > 0) await delay(intervalMs)
    try {
      const task = await api<{ status: string; errorMessage?: string | null }>(
        `/api/v1/tasks/${taskId}`,
      )
      const type = terminalEventForStatus(task.status)
      if (!type) continue
      onEvent({
        id: '',
        type,
        data: { taskId, ...(task.errorMessage ? { errorMessage: task.errorMessage } : {}) },
      })
      return
    } catch (error) {
      if (error instanceof ApiError && [401, 403, 404].includes(error.status)) throw error
    }
  }
  throw new Error('生成进度连接超时，请稍后刷新会话查看任务结果')
}

function terminalEventForStatus(status: string): string | null {
  if (status === 'succeeded') return 'task.completed'
  if (status === 'failed') return 'task.failed'
  if (status === 'cancelled') return 'task.cancelled'
  return null
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds))
}

export function parseEvent(block: string): StreamEvent | null {
  if (!block || block.startsWith(':')) return null
  let id = ''
  let type = 'message'
  const data: string[] = []
  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith('id:')) id = line.slice(3).trim()
    if (line.startsWith('event:')) type = line.slice(6).trim()
    if (line.startsWith('data:')) data.push(line.slice(5).trim())
  }
  if (!data.length) return null
  return { id, type, data: JSON.parse(data.join('\n')) as Record<string, unknown> }
}
