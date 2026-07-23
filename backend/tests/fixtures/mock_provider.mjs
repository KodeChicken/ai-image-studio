import http from 'node:http'
import crypto from 'node:crypto'

const image =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl2sAAAAASUVORK5CYII='
const updaterToken = process.env.MOCK_UPDATER_TOKEN || ''
const updaterJobs = new Map()
let cancelledProviderRequests = 0
let retryAttempts = 0
const port = Number(process.env.MOCK_PROVIDER_PORT || 3401)

function readBody(request) {
  return new Promise((resolve, reject) => {
    const chunks = []
    request.on('data', (chunk) => chunks.push(chunk))
    request.on('end', () => resolve(Buffer.concat(chunks)))
    request.on('error', reject)
  })
}

function updaterRequestIsValid(request, body) {
  const timestamp = request.headers['x-ai-studio-timestamp']
  const signature = request.headers['x-ai-studio-signature']
  if (
    !updaterToken ||
    request.headers.authorization !== `Bearer ${updaterToken}` ||
    typeof timestamp !== 'string' ||
    Math.abs(Math.floor(Date.now() / 1000) - Number(timestamp)) > 60 ||
    typeof signature !== 'string'
  ) {
    return false
  }
  const bodyHash = crypto.createHash('sha256').update(body).digest('hex')
  const payload = `${timestamp}\n${request.method}\n${request.url}\n${bodyHash}`
  const expected = crypto.createHmac('sha256', updaterToken).update(payload).digest()
  const received = Buffer.from(signature, 'hex')
  return received.length === expected.length && crypto.timingSafeEqual(received, expected)
}

function json(response, status, payload) {
  response.writeHead(status, { 'content-type': 'application/json' })
  response.end(JSON.stringify(payload))
}

const server = http.createServer(async (request, response) => {
  if (request.method === 'POST' && request.url === '/updater/v1/jobs') {
    const body = await readBody(request)
    if (!updaterRequestIsValid(request, body)) {
      json(response, 401, { error: { message: 'invalid updater signature' } })
      return
    }
    const job = JSON.parse(body.toString('utf8'))
    updaterJobs.set(job.jobId, job)
    json(response, 202, { ...job, status: 'running', progress: 1 })
    return
  }

  const updaterMatch = request.url?.match(/^\/updater\/v1\/jobs\/([0-9a-f-]+)$/i)
  if (request.method === 'GET' && updaterMatch) {
    const body = Buffer.alloc(0)
    if (!updaterRequestIsValid(request, body)) {
      json(response, 401, { error: { message: 'invalid updater signature' } })
      return
    }
    const job = updaterJobs.get(updaterMatch[1])
    if (!job) {
      json(response, 404, { error: { message: 'job not found' } })
      return
    }
    json(response, 200, {
      status: 'succeeded',
      progress: 100,
      currentStep: 'completed',
      errorMessage: null,
      deployment: {
        appVersion: job.targetVersion,
        imageReference: `ghcr.io/example/ai-image-studio:v${job.targetVersion}`,
        imageDigest: `sha256:${'3'.repeat(64)}`,
        schemaVersion: 9,
        backupReference: `/var/lib/ai-image-studio-updater/backups/${job.jobId}/backup-manifest.json`,
      },
    })
    return
  }

  if (request.method === 'GET' && request.url === '/v1/models') {
    json(response, 200, {
      object: 'list',
      data: [{ id: 'gpt-image-1', object: 'model', created: 1, owned_by: 'test' }],
    })
    return
  }

  if (request.method === 'GET' && request.url === '/test/provider-cancellations') {
    json(response, 200, { count: cancelledProviderRequests })
    return
  }

  if (
    request.method === 'POST' &&
    (request.url === '/v1/images/generations' || request.url === '/v1/images/edits')
  ) {
    const body = await readBody(request)
    let payload = null
    try {
      payload = JSON.parse(body.toString('utf8'))
    } catch {
      // Multipart edit requests are not JSON and use the normal immediate response.
    }
    if (payload?.prompt === 'SLOW_CANCEL_TEST') {
      let completed = false
      response.on('close', () => {
        if (!completed) cancelledProviderRequests += 1
      })
      setTimeout(() => {
        if (!response.destroyed) {
          completed = true
          json(response, 200, { created: 1, data: [{ b64_json: image }] })
        }
      }, 5000)
      return
    }
    if (payload?.prompt?.includes('FAIL_ONCE_RETRY_TEST') && retryAttempts++ === 0) {
      json(response, 503, { error: { message: 'temporary provider failure' } })
      return
    }
    if (payload?.prompt?.includes('MODERATION_BLOCK_TEST')) {
      response.writeHead(400, {
        'content-type': 'application/json',
        'x-request-id': 'req_moderation_test',
      })
      response.end(JSON.stringify({
        error: {
          message: 'The request was rejected by the safety system.',
          type: 'image_generation_user_error',
          code: 'moderation_blocked',
          moderation_details: {
            moderation_stage: 'input',
            categories: ['sexual'],
          },
        },
      }))
      return
    }
    if (payload?.stream === true) {
      response.writeHead(200, { 'content-type': 'text/event-stream' })
      response.write(
        `event: image_generation.partial_image\ndata: ${JSON.stringify({ type: 'image_generation.partial_image', partial_image_index: 0, b64_json: image })}\n\n`,
      )
      response.end(
        `event: image_generation.completed\ndata: ${JSON.stringify({ type: 'image_generation.completed', b64_json: image })}\n\n`,
      )
      return
    }
    json(response, 200, { created: 1, data: [{ b64_json: image }] })
    return
  }

  json(response, 404, { error: { message: 'not found' } })
})

server.listen(port, '127.0.0.1', () => {
  console.log(`mock provider listening on 127.0.0.1:${port}`)
})
