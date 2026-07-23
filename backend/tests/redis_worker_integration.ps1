$ErrorActionPreference = 'Stop'

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$testRoot = Join-Path $workspace ("data\integration-redis-" + [guid]::NewGuid().ToString('N'))
$postgresContainer = "ai-image-studio-redis-pg-" + [guid]::NewGuid().ToString('N').Substring(0, 10)
$redisContainer = "ai-image-studio-redis-queue-" + [guid]::NewGuid().ToString('N').Substring(0, 10)
$queueKey = "ai-image-studio:test:" + [guid]::NewGuid().ToString('N')
$appPort = 3311
$mockPort = 3402
$postgresPort = 55434
$redisPort = 56379
$binaryName = if ($IsWindows) { 'ai-image-studio.exe' } else { 'ai-image-studio' }
$binaryPath = [IO.Path]::Combine($workspace, 'target', 'debug', $binaryName)
$mockProcess = $null
$appProcess = $null
$workerProcess = $null
$workerSequence = 0

function Get-FreePort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Start-HiddenProcess {
    param(
        [string] $FilePath,
        [string[]] $ArgumentList,
        [string] $WorkingDirectory,
        [string] $StandardOutput,
        [string] $StandardError
    )

    $arguments = @{
        FilePath = $FilePath
        ArgumentList = $ArgumentList
        WorkingDirectory = $WorkingDirectory
        PassThru = $true
        RedirectStandardOutput = $StandardOutput
        RedirectStandardError = $StandardError
    }
    if ($IsWindows) {
        $arguments.WindowStyle = 'Hidden'
    }
    Start-Process @arguments
}

function Start-TestWorker {
    $script:workerSequence += 1
    Start-HiddenProcess `
        -FilePath $binaryPath `
        -ArgumentList @('worker') `
        -WorkingDirectory $workspace `
        -StandardOutput (Join-Path $testRoot "worker-$workerSequence.out.log") `
        -StandardError (Join-Path $testRoot "worker-$workerSequence.err.log")
}

function Wait-CompletedTask {
    param(
        [string] $TaskId,
        [int] $Attempts = 80
    )

    for ($attempt = 0; $attempt -lt $Attempts; $attempt++) {
        $task = Invoke-RestMethod `
            -Uri ("http://127.0.0.1:{0}/api/v1/tasks/{1}" -f $appPort, $TaskId) `
            -WebSession $session
        if ($task.status -eq 'succeeded') {
            return $task
        }
        if ($task.status -eq 'failed' -or $task.status -eq 'cancelled') {
            throw "Task $TaskId ended as $($task.status): $($task.errorMessage)"
        }
        Start-Sleep -Milliseconds 250
    }
    throw "Task $TaskId did not complete"
}

function New-TestTask {
    param([string] $Content)

    Invoke-RestMethod `
        -Uri ("http://127.0.0.1:{0}/api/v1/conversations/{1}/messages" -f $appPort, $conversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = $Content
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
}

$appPort = Get-FreePort
$mockPort = Get-FreePort
$postgresPort = Get-FreePort
$redisPort = Get-FreePort
New-Item -ItemType Directory -Path (Join-Path $testRoot 'images') -Force | Out-Null

try {
    Push-Location $workspace
    try {
        cargo build --package ai-image-studio
        if ($LASTEXITCODE -ne 0) {
            throw 'Backend build failed'
        }
    }
    finally {
        Pop-Location
    }

    docker run --name $postgresContainer `
        -e POSTGRES_DB=studio_redis_test `
        -e POSTGRES_USER=studio_redis_test `
        -e POSTGRES_PASSWORD=studio_redis_test `
        -p ("127.0.0.1:{0}:5432" -f $postgresPort) `
        -d postgres:17-alpine | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to start PostgreSQL container'
    }
    docker run --name $redisContainer `
        -p ("127.0.0.1:{0}:6379" -f $redisPort) `
        -d redis:8-alpine redis-server --appendonly no | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to start Redis container'
    }

    $postgresReady = $false
    $redisReady = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        if (-not $postgresReady) {
            docker exec $postgresContainer pg_isready -U studio_redis_test -d studio_redis_test 2>$null | Out-Null
            $postgresReady = $LASTEXITCODE -eq 0
        }
        if (-not $redisReady) {
            $redisReady = (docker exec $redisContainer redis-cli ping 2>$null) -eq 'PONG'
        }
        if ($postgresReady -and $redisReady) {
            break
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $postgresReady -or -not $redisReady) {
        throw 'PostgreSQL or Redis did not become ready'
    }

    $env:MOCK_PROVIDER_PORT = [string]$mockPort
    $mockProcess = Start-HiddenProcess `
        -FilePath 'node' `
        -ArgumentList @('backend/tests/fixtures/mock_provider.mjs') `
        -WorkingDirectory $workspace `
        -StandardOutput (Join-Path $testRoot 'mock.out.log') `
        -StandardError (Join-Path $testRoot 'mock.err.log')

    $env:APP_ENV = 'development'
    $env:LISTEN_ADDR = "127.0.0.1:$appPort"
    $env:STATIC_DIR = Join-Path $workspace 'frontend\dist'
    $env:DATABASE_URL = "postgres://studio_redis_test:studio_redis_test@127.0.0.1:$postgresPort/studio_redis_test"
    $env:SESSION_SECRET = 'redis-integration-session-secret-at-least-32-characters'
    $env:CREDENTIAL_MASTER_KEY = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    $env:STORAGE_DRIVER = 'local'
    $env:STORAGE_LOCAL_PATH = Join-Path $testRoot 'images'
    $env:STORAGE_CONSISTENCY_SCAN_ENABLED = 'false'
    $env:TASK_EXECUTION_MODE = 'redis'
    $env:REDIS_URL = "redis://127.0.0.1:$redisPort/0"
    $env:TASK_QUEUE_KEY = $queueKey
    $env:TASK_MAX_RETRIES = '1'
    $env:TASK_RETRY_DELAY_SECONDS = '1'
    $env:RATE_LIMIT_ENABLED = 'false'
$env:ALLOW_CUSTOM_BASE_URL = 'true'
$env:ALLOW_PRIVATE_PROVIDER_HOSTS = 'true'
    $env:BOOTSTRAP_ADMIN_ENABLED = 'true'
    $env:BOOTSTRAP_ADMIN_USERNAME = 'admin'
    $env:BOOTSTRAP_ADMIN_PASSWORD = '123456'
    $env:BOOTSTRAP_ADMIN_FORCE_PASSWORD_CHANGE = 'true'
    $env:RUST_LOG = 'ai_image_studio=warn'

    $appProcess = Start-HiddenProcess `
        -FilePath $binaryPath `
        -ArgumentList @('serve') `
        -WorkingDirectory $workspace `
        -StandardOutput (Join-Path $testRoot 'app.out.log') `
        -StandardError (Join-Path $testRoot 'app.err.log')

    $apiReady = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$appPort/api/v1/health"
            if ($health.status -eq 'ok') {
                $apiReady = $true
                break
            }
        }
        catch {
            # Migrations and bootstrap can still be running.
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $apiReady) {
        throw 'Application did not become ready'
    }

    $login = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/auth/login" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ username = 'admin'; password = '123456' } | ConvertTo-Json) `
        -SessionVariable session
    if (-not $login.mustChangePassword) {
        throw 'Bootstrap password change was not required'
    }
    Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/users/me/change-password" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ currentPassword = '123456'; newPassword = 'RedisIntegration123!' } | ConvertTo-Json) `
        -WebSession $session | Out-Null

    $provider = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/providers" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            providerKey = 'redis-mock'
            providerType = 'openai-compatible'
            displayName = 'Redis Worker Mock'
            baseUrl = "http://127.0.0.1:$mockPort/v1"
            apiKey = 'test-key'
            config = @{}
        } | ConvertTo-Json) `
        -WebSession $session
    Invoke-RestMethod `
        -Uri ("http://127.0.0.1:{0}/api/v1/providers/{1}/models/discover" -f $appPort, $provider.id) `
        -Method Post `
        -WebSession $session | Out-Null
    $model = @(Invoke-RestMethod `
            -Uri "http://127.0.0.1:$appPort/api/v1/models?includeDiscovered=true" `
            -WebSession $session)[0]
    $model = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:{0}/api/v1/providers/{1}/models/{2}" -f $appPort, $provider.id, $model.id) `
        -Method Post `
        -WebSession $session
    if ($model.availabilityStatus -ne 'verified') {
        throw 'Model did not become verified'
    }
    $conversation = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/conversations" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Redis worker integration'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session

    $queued = New-TestTask -Content 'REDIS_QUEUE_TEST'
    Start-Sleep -Milliseconds 500
    $pending = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:{0}/api/v1/tasks/{1}" -f $appPort, $queued.taskId) `
        -WebSession $session
    $queueLength = [int](docker exec $redisContainer redis-cli LLEN $queueKey)
    if ($pending.status -ne 'pending' -or $queueLength -ne 1) {
        throw 'API did not leave the task pending in Redis before the worker started'
    }

    $workerProcess = Start-TestWorker
    $completed = Wait-CompletedTask -TaskId $queued.taskId
    if (@($completed.results).Count -ne 1) {
        throw 'Redis worker did not persist the generated result'
    }
    $content = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:{0}{1}" -f $appPort, $completed.results[0].contentUrl) `
        -WebSession $session
    if ($content.StatusCode -ne 200 -or $content.Headers.'Content-Type' -notmatch '^image/png') {
        throw 'Redis worker result content is unavailable'
    }

    Stop-Process -Id $workerProcess.Id -Force
    Wait-Process -Id $workerProcess.Id -ErrorAction SilentlyContinue
    $workerProcess = $null
    $fallback = New-TestTask -Content 'POSTGRES_FALLBACK_TEST'
    $queuedForFallback = [int](docker exec $redisContainer redis-cli LLEN $queueKey)
    if ($queuedForFallback -ne 1) {
        throw 'PostgreSQL fallback task was not initially queued in Redis'
    }
    docker exec $redisContainer redis-cli DEL $queueKey | Out-Null
    $workerProcess = Start-TestWorker
    $fallbackCompleted = Wait-CompletedTask -TaskId $fallback.taskId -Attempts 100
    if ($fallbackCompleted.status -ne 'succeeded') {
        throw 'Worker did not recover the missing Redis message from PostgreSQL'
    }

    $retry = New-TestTask -Content 'FAIL_ONCE_RETRY_TEST'
    $retryCompleted = Wait-CompletedTask -TaskId $retry.taskId -Attempts 100
    if ($retryCompleted.retryCount -ne 1) {
        throw "Expected one automatic retry, got $($retryCompleted.retryCount)"
    }
    $retryEvents = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:{0}/api/v1/tasks/{1}/events" -f $appPort, $retry.taskId) `
        -Headers @{ Accept = 'text/event-stream' } `
        -WebSession $session
    if ($retryEvents.Content -notmatch '"stage":"automatic_retry"') {
        throw 'Automatic retry did not emit a persisted task.progress event'
    }

    $finalQueueLength = [int](docker exec $redisContainer redis-cli LLEN $queueKey)
    if ($finalQueueLength -ne 0) {
        throw "Redis queue was not drained, remaining length $finalQueueLength"
    }
    Write-Output 'REDIS_WORKER_INTEGRATION_OK api_does_not_execute=1 redis_consume=1 postgres_fallback=1 automatic_retry=1 durable_image=1'
}
finally {
    foreach ($process in @($workerProcess, $appProcess, $mockProcess)) {
        if ($process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if ($postgresContainer.StartsWith('ai-image-studio-redis-pg-')) {
        docker rm -f $postgresContainer 2>$null | Out-Null
    }
    if ($redisContainer.StartsWith('ai-image-studio-redis-queue-')) {
        docker rm -f $redisContainer 2>$null | Out-Null
    }
    $resolvedData = (Resolve-Path (Join-Path $workspace 'data')).Path
    if (
        (Test-Path -LiteralPath $testRoot) -and
        $testRoot.StartsWith($resolvedData + [IO.Path]::DirectorySeparatorChar) -and
        (Split-Path $testRoot -Leaf).StartsWith('integration-redis-')
    ) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
