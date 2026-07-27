$ErrorActionPreference = 'Stop'

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$testRoot = Join-Path $workspace ("data\integration-core-" + [guid]::NewGuid().ToString('N'))
$containerName = "ai-image-studio-core-test-" + [guid]::NewGuid().ToString('N').Substring(0, 10)
$updaterToken = 'integration-host-updater-token-1234567890'
$mockProcess = $null
$appProcess = $null

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

    docker run --name $containerName `
        -e POSTGRES_DB=studio_test `
        -e POSTGRES_USER=studio_test `
        -e POSTGRES_PASSWORD=studio_test `
        -p 127.0.0.1:55433:5432 `
        -d postgres:17-alpine | Out-Null

    $dbReady = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        docker exec $containerName pg_isready -U studio_test -d studio_test 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) {
            $dbReady = $true
            break
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $dbReady) {
        throw 'PostgreSQL did not become ready'
    }

    $env:MOCK_UPDATER_TOKEN = $updaterToken
    $mockProcess = Start-Process `
        -FilePath 'node' `
        -ArgumentList 'backend/tests/fixtures/mock_provider.mjs' `
        -WorkingDirectory $workspace `
        -WindowStyle Hidden `
        -PassThru `
        -RedirectStandardOutput (Join-Path $testRoot 'mock.out.log') `
        -RedirectStandardError (Join-Path $testRoot 'mock.err.log')

    $env:APP_ENV = 'development'
    $env:LISTEN_ADDR = '127.0.0.1:3310'
    $env:STATIC_DIR = Join-Path $workspace 'frontend\dist'
    $env:DATABASE_URL = 'postgres://studio_test:studio_test@127.0.0.1:55433/studio_test'
    $env:SESSION_SECRET = 'integration-session-secret-at-least-32-characters'
    $env:CREDENTIAL_MASTER_KEY = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    $env:STORAGE_DRIVER = 'local'
    $env:STORAGE_LOCAL_PATH = Join-Path $testRoot 'images'
    $env:STORAGE_CONSISTENCY_SCAN_ENABLED = 'true'
    $env:STORAGE_CONSISTENCY_SCAN_INTERVAL_SECONDS = '86400'
    $env:STORAGE_ORPHAN_GRACE_SECONDS = '1'
    $env:TASK_EXECUTION_MODE = 'inline'
    $env:REDIS_URL = ''
    $env:TASK_MAX_RETRIES = '0'
    $env:RATE_LIMIT_ENABLED = 'true'
    $env:RATE_LIMIT_WINDOW_SECONDS = '60'
    $env:RATE_LIMIT_IP_REQUESTS = '300'
    $env:RATE_LIMIT_SESSION_REQUESTS = '260'
    $env:RATE_LIMIT_USER_REQUESTS = '240'
    $env:HOST_UPDATER_URL = 'http://127.0.0.1:3401/updater/'
    $env:HOST_UPDATER_SOCKET = ''
    $env:HOST_UPDATER_TOKEN = $updaterToken
    $env:ALLOW_CUSTOM_BASE_URL = 'true'
    $env:ALLOW_PRIVATE_PROVIDER_HOSTS = 'true'
    $env:BOOTSTRAP_ADMIN_ENABLED = 'true'
    $env:BOOTSTRAP_ADMIN_USERNAME = 'admin'
    $env:BOOTSTRAP_ADMIN_PASSWORD = '123456'
    $env:BOOTSTRAP_ADMIN_FORCE_PASSWORD_CHANGE = 'true'
    $env:RUST_LOG = 'ai_image_studio=warn'

    $appProcess = Start-Process `
        -FilePath (Join-Path $workspace 'target\debug\ai-image-studio.exe') `
        -ArgumentList 'serve' `
        -WorkingDirectory $workspace `
        -WindowStyle Hidden `
        -PassThru `
        -RedirectStandardOutput (Join-Path $testRoot 'app.out.log') `
        -RedirectStandardError (Join-Path $testRoot 'app.err.log')

    $apiReady = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        try {
            $health = Invoke-RestMethod -Uri 'http://127.0.0.1:3310/api/v1/health'
            if ($health.status -eq 'ok') {
                $apiReady = $true
                break
            }
        }
        catch {
            # The server can refuse connections while migrations are running.
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $apiReady) {
        throw 'Application did not become ready'
    }

    $loginBody = @{ username = 'admin'; password = '123456' } | ConvertTo-Json
    $login = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/auth/login' `
        -Method Post `
        -ContentType 'application/json' `
        -Body $loginBody `
        -SessionVariable session `
        -ResponseHeadersVariable loginHeaders
    if (-not $login.mustChangePassword) {
        throw 'Bootstrap password-change requirement was not enforced'
    }
    $loginSetCookie = @($loginHeaders['Set-Cookie']) -join '; '
    foreach ($requiredCookieAttribute in @('HttpOnly', 'SameSite=Lax', 'Path=/', 'Max-Age=')) {
        if ($loginSetCookie -notmatch [regex]::Escape($requiredCookieAttribute)) {
            throw "Login cookie is missing $requiredCookieAttribute"
        }
    }
    if ($loginSetCookie -match '(?i)(?:^|;\s*)Secure(?:;|$)') {
        throw 'Development login cookie unexpectedly contains Secure'
    }

    $passwordBody = @{
        currentPassword = '123456'
        newPassword = 'Integration123!'
    } | ConvertTo-Json
    Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/users/me/change-password' `
        -Method Post `
        -ContentType 'application/json' `
        -Body $passwordBody `
        -WebSession $session | Out-Null

    $seedDeployment = "INSERT INTO deployment_history (app_version, image_reference, image_digest, schema_version, backup_reference, deployment_status) VALUES ('0.0.9', 'ghcr.io/example/ai-image-studio:v0.0.9', 'sha256:' || repeat('2', 64), 9, '/backups/previous.json', 'superseded')"
    docker exec $containerName psql -U studio_test -d studio_test -v ON_ERROR_STOP=1 -c $seedDeployment | Out-Null
    $updateJob = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/updates/jobs' `
        -Method Post `
        -Headers @{ 'X-AI-Studio-Action' = 'update' } `
        -ContentType 'application/json' `
        -Body (@{
            action = 'rollback'
            targetVersion = '0.0.9'
            currentPassword = 'Integration123!'
            confirmDestructiveMigration = $false
        } | ConvertTo-Json) `
        -WebSession $session
    $syncedUpdateJob = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/admin/updates/jobs/{0}" -f $updateJob.id) `
        -WebSession $session
    if ($syncedUpdateJob.status -ne 'succeeded' -or $syncedUpdateJob.progress -ne 100) {
        throw 'Signed Host Updater job did not synchronize its terminal state'
    }
    $deploymentSourceJob = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT source_job_id FROM deployment_history WHERE deployment_status = 'active' AND app_version = '0.0.9' ORDER BY deployed_at DESC LIMIT 1")
    if ($deploymentSourceJob.Trim() -ne $updateJob.id) {
        throw 'Host Updater deployment evidence was not linked idempotently to its update job'
    }

    $providerBody = @{
        providerKey = 'mock'
        providerType = 'openai-compatible'
        displayName = 'Mock Provider'
        baseUrl = 'http://127.0.0.1:3401/v1'
        apiKey = 'test-key'
        config = @{}
    } | ConvertTo-Json
    $provider = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/providers' `
        -Method Post `
        -ContentType 'application/json' `
        -Body $providerBody `
        -WebSession $session

    $credentialEvidenceRaw = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT json_build_object('ciphertextHex', encode(credential_ciphertext, 'hex'), 'nonceHex', encode(credential_nonce, 'hex'), 'nonceLength', octet_length(credential_nonce), 'keyVersion', credential_key_version, 'config', config_json)::text FROM providers WHERE id = '{0}'" -f $provider.id)
    $credentialEvidenceJson = ($credentialEvidenceRaw -join "`n").Trim()
    $credentialEvidence = $credentialEvidenceJson | ConvertFrom-Json
    $plaintextApiKeyHex = '746573742d6b6579'
    if (
        [string]::IsNullOrWhiteSpace($credentialEvidence.ciphertextHex) -or
        $credentialEvidence.nonceLength -ne 12 -or
        $credentialEvidence.keyVersion -ne 1 -or
        $credentialEvidence.ciphertextHex -match $plaintextApiKeyHex -or
        $credentialEvidenceJson -match [regex]::Escape('test-key')
    ) {
        throw 'Provider credential was not stored as AES-GCM ciphertext with the expected metadata'
    }
    $providerCreateJson = $provider | ConvertTo-Json -Depth 10 -Compress
    if (
        -not $provider.credentialConfigured -or
        $providerCreateJson -match [regex]::Escape('test-key') -or
        $providerCreateJson -match [regex]::Escape($credentialEvidence.ciphertextHex) -or
        $providerCreateJson -match [regex]::Escape($credentialEvidence.nonceHex)
    ) {
        throw 'Provider create response exposed credential material or omitted credential state'
    }

    $healthResult = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/test" -f $provider.id) `
        -Method Post `
        -WebSession $session
    if ($healthResult.status -ne 'healthy' -or $healthResult.modelCount -ne 1) {
        throw 'Provider health test returned an unexpected result'
    }
    $providers = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/providers' `
        -WebSession $session
    if (@($providers)[0].healthStatus -ne 'healthy') {
        throw 'Provider health state was not persisted'
    }
    $providerListJson = $providers | ConvertTo-Json -Depth 10 -Compress
    if (
        $providerListJson -match [regex]::Escape('test-key') -or
        $providerListJson -match [regex]::Escape($credentialEvidence.ciphertextHex) -or
        $providerListJson -match [regex]::Escape($credentialEvidence.nonceHex)
    ) {
        throw 'Provider list response exposed credential material'
    }

    Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/discover" -f $provider.id) `
        -Method Post `
        -WebSession $session | Out-Null
    $models = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/models?includeDiscovered=true' `
        -WebSession $session
    $model = @($models)[0]
    $verified = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}" -f $provider.id, $model.id) `
        -Method Post `
        -WebSession $session
    if ($verified.availabilityStatus -ne 'verified') {
        throw 'Model verification did not persist verified status'
    }
    $testGeneration = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/test-generation" -f $provider.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            modelId = $model.id
            prompt = 'A purple glass sphere on a white background.'
            parameters = @{ n = 1; size = 'auto'; quality = 'auto' }
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    if (
        $testGeneration.imageDataUrl -notmatch '^data:image/(png|jpeg|webp);base64,' -or
        $testGeneration.width -lt 1 -or
        $testGeneration.height -lt 1
    ) {
        throw 'Provider test generation did not return a displayable image'
    }

    $priceBody = @{ price = '0.125'; currency = 'cny' } | ConvertTo-Json
    $price = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}/pricing" -f $provider.id, $model.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body $priceBody `
        -WebSession $session
    $prices = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}/pricing" -f $provider.id, $model.id) `
        -WebSession $session
    if (@($prices).Count -ne 1 -or @($prices)[0].currency -ne 'CNY') {
        throw 'Model pricing create/list failed'
    }
    $overlapResponse = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}/pricing" -f $provider.id, $model.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body $priceBody `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($overlapResponse.StatusCode -ne 409) {
        throw "Expected overlapping model pricing to return 409, got $($overlapResponse.StatusCode)"
    }

    function Wait-CompletedTask {
        param([string] $TaskId)

        $task = $null
        for ($attempt = 0; $attempt -lt 40; $attempt++) {
            $task = Invoke-RestMethod `
                -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $TaskId) `
                -WebSession $session
            if ($task.status -eq 'succeeded') {
                return $task
            }
            if ($task.status -eq 'failed') {
                throw "Generation task failed: $($task.errorMessage)"
            }
            Start-Sleep -Milliseconds 250
        }
        throw 'Generation task did not complete'
    }

    function New-CompletedTask {
        param([string] $Title)

        $conversation = Invoke-RestMethod `
            -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
            -Method Post `
            -ContentType 'application/json' `
            -Body (@{
                title = $Title
                defaultProviderId = $provider.id
                defaultModelId = $model.id
            } | ConvertTo-Json) `
            -WebSession $session
        $created = Invoke-RestMethod `
            -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $conversation.id) `
            -Method Post `
            -ContentType 'application/json' `
            -Body (@{
                content = 'Create a test image'
                providerId = $provider.id
                modelId = $model.id
                stream = $false
                parameters = @{ size = 'auto'; n = 1 }
                inputAssetIds = @()
            } | ConvertTo-Json -Depth 5) `
            -WebSession $session

        $task = Wait-CompletedTask -TaskId $created.taskId
        return @{ Conversation = $conversation; Task = $task }
    }

    $uploadFixture = Join-Path $testRoot 'input.png'
    [IO.File]::WriteAllBytes(
        $uploadFixture,
        [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl2sAAAAASUVORK5CYII=')
    )
    function New-TestUpload {
        Invoke-RestMethod `
            -Uri 'http://127.0.0.1:3310/api/v1/image-assets/uploads' `
            -Method Post `
            -Form @{ file = Get-Item -LiteralPath $uploadFixture } `
            -WebSession $session
    }

    $discardedUpload = New-TestUpload
    $discardedStorageKey = (docker exec $containerName `
            psql -U studio_test -d studio_test -tAc `
            ("SELECT storage_key FROM image_assets WHERE id = '{0}'" -f $discardedUpload.id)).Trim()
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}" -f $discardedUpload.id) `
        -Method Delete `
        -WebSession $session | Out-Null
    $discardedContent = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}/content" -f $discardedUpload.id) `
        -WebSession $session `
        -SkipHttpErrorCheck
    $discardedFile = Join-Path (Join-Path $testRoot 'images') ($discardedStorageKey -replace '/', [IO.Path]::DirectorySeparatorChar)
    if ($discardedContent.StatusCode -ne 404 -or (Test-Path -LiteralPath $discardedFile)) {
        throw 'Deleting an unreferenced upload did not remove its database row and real file'
    }

    $failedInput = New-TestUpload
    $failedInputStorageKey = (docker exec $containerName `
            psql -U studio_test -d studio_test -tAc `
            ("SELECT storage_key FROM image_assets WHERE id = '{0}'" -f $failedInput.id)).Trim()
    $failedInputConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Failed input compensation test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $failedInputResponse = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $failedInputConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'This invalid task must compensate its upload'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ unsupported_parameter = 1 }
            inputAssetIds = @($failedInput.id)
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session `
        -SkipHttpErrorCheck
    $failedInputCount = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT COUNT(*) FROM image_assets WHERE id = '{0}'" -f $failedInput.id)
    $failedInputFile = Join-Path (Join-Path $testRoot 'images') ($failedInputStorageKey -replace '/', [IO.Path]::DirectorySeparatorChar)
    if (
        $failedInputResponse.StatusCode -ne 400 -or
        $failedInputCount.Trim() -ne '0' -or
        (Test-Path -LiteralPath $failedInputFile)
    ) {
        throw 'Task creation failure did not compensate its unreferenced input Asset and file'
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $failedInputConversation.id) `
        -Method Delete `
        -WebSession $session | Out-Null

    $referencedInput = New-TestUpload
    $referencedConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Referenced input protection test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $referencedCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $referencedConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'Edit this referenced input'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @($referencedInput.id)
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    $referencedTask = Wait-CompletedTask -TaskId $referencedCreated.taskId
    $referencedDelete = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}" -f $referencedInput.id) `
        -Method Delete `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($referencedTask.operation -ne 'edit' -or $referencedDelete.StatusCode -ne 409) {
        throw 'An input Asset linked to a completed task was not protected from direct deletion'
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $referencedConversation.id) `
        -Method Delete `
        -WebSession $session | Out-Null

    $cancelConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Provider cancellation test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $cancelCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $cancelConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'SLOW_CANCEL_TEST'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    $processingObserved = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $cancelTask = Invoke-RestMethod `
            -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $cancelCreated.taskId) `
            -WebSession $session
        if ($cancelTask.status -eq 'processing') {
            $processingObserved = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $processingObserved) {
        throw 'Slow provider task never entered processing before cancellation'
    }
    $cancelStopwatch = [Diagnostics.Stopwatch]::StartNew()
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/cancel" -f $cancelCreated.taskId) `
        -Method Post `
        -WebSession $session | Out-Null
    $providerCancellationObserved = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $cancelStats = Invoke-RestMethod -Uri 'http://127.0.0.1:3401/test/provider-cancellations'
        if ($cancelStats.count -ge 1) {
            $providerCancellationObserved = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    $cancelStopwatch.Stop()
    if (-not $providerCancellationObserved -or $cancelStopwatch.Elapsed.TotalSeconds -ge 3) {
        throw 'Cancelling a processing task did not promptly abort its upstream HTTP request'
    }
    $cancelledTask = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $cancelCreated.taskId) `
        -WebSession $session
    if ($cancelledTask.status -ne 'cancelled' -or @($cancelledTask.results).Count -ne 0) {
        throw 'Cancelled task reached an invalid terminal state or retained partial results'
    }
    $cancelledConversation = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $cancelConversation.id) `
        -WebSession $session
    $cancelledAssistant = @($cancelledConversation.messages | Where-Object id -eq $cancelCreated.messageId)
    if (
        $cancelledAssistant.Count -ne 1 -or
        $cancelledAssistant[0].status -ne 'cancelled' -or
        $cancelledAssistant[0].content -ne '生成已取消'
    ) {
        throw 'Cancelled task did not update its assistant message terminal state'
    }
    $cancelEvents = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/events" -f $cancelCreated.taskId) `
        -Headers @{ Accept = 'text/event-stream' } `
        -WebSession $session
    if ($cancelEvents.Content -notmatch 'event: task.cancelled') {
        throw 'Cancelled task did not emit the task.cancelled terminal event'
    }

    $retryConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Manual retry test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $retryCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $retryConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'FAIL_ONCE_RETRY_TEST'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    $failedRetryTask = $null
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $failedRetryTask = Invoke-RestMethod `
            -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $retryCreated.taskId) `
            -WebSession $session
        if ($failedRetryTask.status -eq 'failed') {
            break
        }
        Start-Sleep -Milliseconds 250
    }
    if ($failedRetryTask.status -ne 'failed' -or -not $failedRetryTask.errorMessage) {
        throw 'Retry test task did not preserve its first failure summary'
    }
    $failedRetryConversation = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $retryConversation.id) `
        -WebSession $session
    $failedRetryAssistant = @($failedRetryConversation.messages | Where-Object id -eq $retryCreated.messageId)
    if (
        $failedRetryAssistant.Count -ne 1 -or
        $failedRetryAssistant[0].status -ne 'failed' -or
        $failedRetryAssistant[0].taskId -ne $retryCreated.taskId -or
        -not $failedRetryAssistant[0].taskErrorMessage
    ) {
        throw 'Failed assistant message did not expose its retry task and error summary'
    }
    $manualRetry = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/retry" -f $retryCreated.taskId) `
        -Method Post `
        -WebSession $session
    if ($manualRetry.taskId -ne $retryCreated.taskId -or [int64]$manualRetry.lastEventId -le 0) {
        throw 'Manual retry did not return a resumable event cursor for the same task'
    }
    $manualRetryEvents = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/events" -f $retryCreated.taskId) `
        -Headers @{ Accept = 'text/event-stream'; 'Last-Event-ID' = [string]$manualRetry.lastEventId } `
        -WebSession $session
    if (
        $manualRetryEvents.Content -notmatch 'event: task.completed' -or
        $manualRetryEvents.Content -match 'event: task.failed'
    ) {
        throw 'Manual retry SSE replay included the old failure or missed the new completion'
    }
    $retriedTask = Wait-CompletedTask -TaskId $retryCreated.taskId
    if ($retriedTask.retryCount -ne 1 -or @($retriedTask.results).Count -ne 1) {
        throw 'Manual retry did not complete the original task with one persisted result'
    }
    $completedRetryConversation = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $retryConversation.id) `
        -WebSession $session
    $completedRetryAssistant = @($completedRetryConversation.messages | Where-Object id -eq $retryCreated.messageId)
    if (
        $completedRetryAssistant.Count -ne 1 -or
        $completedRetryAssistant[0].status -ne 'completed' -or
        $completedRetryAssistant[0].taskRetryCount -ne 1 -or
        $completedRetryAssistant[0].taskErrorMessage
    ) {
        throw 'Retried assistant message did not reload as a clean completed result'
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $retryConversation.id) `
        -Method Delete `
        -WebSession $session | Out-Null

    $rejectedConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Provider rejection test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $rejectedCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $rejectedConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'MODERATION_BLOCK_TEST'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    $rejectedTask = $null
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $rejectedTask = Invoke-RestMethod `
            -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $rejectedCreated.taskId) `
            -WebSession $session
        if ($rejectedTask.status -eq 'failed') {
            break
        }
        Start-Sleep -Milliseconds 250
    }
    if (
        $rejectedTask.status -ne 'failed' -or
        $rejectedTask.errorCode -ne 'moderation_blocked' -or
        $rejectedTask.retryCount -ne 0 -or
        $rejectedTask.errorMessage -notmatch '安全检查'
    ) {
        throw 'Provider rejection was not returned as a non-retryable user-facing task error'
    }
    $rejectedConversationState = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $rejectedConversation.id) `
        -WebSession $session
    $rejectedAssistant = @($rejectedConversationState.messages | Where-Object id -eq $rejectedCreated.messageId)
    if (
        $rejectedAssistant.Count -ne 1 -or
        $rejectedAssistant[0].status -ne 'failed' -or
        $rejectedAssistant[0].taskErrorMessage -notmatch '安全检查'
    ) {
        throw 'Provider rejection reason was not exposed on the assistant message'
    }
    $rejectedLog = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT status_code || '|' || error_code || '|' || error_summary FROM request_logs WHERE task_id = '{0}' ORDER BY id DESC LIMIT 1" -f $rejectedCreated.taskId)
    if (
        $rejectedLog -notmatch '^400\|moderation_blocked\|' -or
        $rejectedLog -notmatch 'request_id=req_moderation_test'
    ) {
        throw 'Provider rejection diagnostics did not preserve the upstream status, code and request ID'
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $rejectedConversation.id) `
        -Method Delete `
        -WebSession $session | Out-Null

    $first = New-CompletedTask -Title 'History delete test'
    $history = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/history' `
        -WebSession $session
    if (@($history).Count -ne 1) {
        throw 'Expected one history result before deletion'
    }
    $historyItem = @($history)[0]
    $historyNow = [DateTimeOffset]::UtcNow
    $historyFrom = [uri]::EscapeDataString($historyNow.AddMinutes(-5).ToString('o'))
    $historyTo = [uri]::EscapeDataString($historyNow.AddMinutes(5).ToString('o'))
    $filteredHistoryUri = "http://127.0.0.1:3310/api/v1/history?conversationId={0}&providerId={1}&modelId={2}&createdFrom={3}&createdTo={4}&width={5}&height={6}" -f $first.Conversation.id, $provider.id, $model.id, $historyFrom, $historyTo, $historyItem.width, $historyItem.height
    $filteredHistory = Invoke-RestMethod `
        -Uri $filteredHistoryUri `
        -WebSession $session
    if (@($filteredHistory).Count -ne 1 -or @($filteredHistory)[0].assetId -ne $historyItem.assetId) {
        throw ("History filters returned unexpected data: uri={0}, expected={1}, actual={2}" -f $filteredHistoryUri, $historyItem.assetId, ($filteredHistory | ConvertTo-Json -Compress))
    }
    $wrongSizeHistory = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/history?width={0}&height={1}" -f ([int]$historyItem.width + 1), $historyItem.height) `
        -WebSession $session
    if (@($wrongSizeHistory).Count -ne 0) {
        throw 'History exact size filter returned a mismatched asset'
    }
    $partialSizeResponse = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/history?width={0}" -f $historyItem.width) `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($partialSizeResponse.StatusCode -ne 400) {
        throw "History partial size filter expected 400, got $($partialSizeResponse.StatusCode)"
    }
    $invalidHistoryRange = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/history?createdFrom={0}&createdTo={1}" -f $historyTo, $historyFrom) `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($invalidHistoryRange.StatusCode -ne 400) {
        throw "History inverted date range expected 400, got $($invalidHistoryRange.StatusCode)"
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/history/{0}" -f $first.Task.id) `
        -Method Delete `
        -WebSession $session | Out-Null
    $historyAfterTaskDelete = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/history' `
        -WebSession $session
    if (@($historyAfterTaskDelete).Count -ne 0) {
        throw 'History task deletion did not remove its result'
    }

    $second = New-CompletedTask -Title 'Conversation delete test'
    $followUpCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $second.Conversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = '保持主体，改成夜景'
            parentMessageId = $second.Task.assistantMessageId
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    $followUpTask = Wait-CompletedTask -TaskId $followUpCreated.taskId
    if (
        $followUpTask.operation -ne 'edit' -or
        @($followUpTask.requestParams.context_asset_ids).Count -ne 1 -or
        @($followUpTask.requestParams.context_asset_ids)[0] -ne $second.Task.results[0].id -or
        $followUpTask.requestParams.context_message_count -lt 2
    ) {
        throw 'Multi-turn follow-up did not use the previous generated image and text context'
    }
    $conversationDetail = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $second.Conversation.id) `
        -WebSession $session
    $followUpUserMessage = @($conversationDetail.messages | Where-Object id -eq $followUpTask.userMessageId)[0]
    $followUpAssistantMessage = @($conversationDetail.messages | Where-Object id -eq $followUpTask.assistantMessageId)[0]
    if (
        @($followUpUserMessage.assets).Count -ne 1 -or
        $followUpUserMessage.assets[0].relationType -ne 'reference' -or
        @($followUpAssistantMessage.assets).Count -ne 1 -or
        $followUpAssistantMessage.assets[0].relationType -ne 'generated'
    ) {
        throw 'Conversation detail did not preserve message image relation types'
    }
    $storedPrompt = (docker exec $containerName `
            psql -U studio_test -d studio_test -tAc `
            ("SELECT prompt FROM image_tasks WHERE id = '{0}'" -f $followUpTask.id)) -join "`n"
    if ($storedPrompt -notmatch 'Conversation context:' -or $storedPrompt -notmatch 'Current request:') {
        throw 'Multi-turn prompt does not contain the bounded conversation context'
    }

    $eventsResponse = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/events" -f $followUpTask.id) `
        -Headers @{ Accept = 'text/event-stream' } `
        -WebSession $session
    if ($eventsResponse.Content -notmatch 'event: task.completed') {
        throw 'SSE replay did not contain the terminal task.completed event'
    }
    $eventIds = [regex]::Matches($eventsResponse.Content, '(?m)^id:\s*(\d+)')
    if ($eventIds.Count -lt 2) {
        throw 'SSE replay did not expose resumable event IDs'
    }
    $resumeAfter = [int64]$eventIds[0].Groups[1].Value
    $resumedResponse = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/events" -f $followUpTask.id) `
        -Headers @{ Accept = 'text/event-stream'; 'Last-Event-ID' = $resumeAfter } `
        -WebSession $session
    $resumedIds = [regex]::Matches($resumedResponse.Content, '(?m)^id:\s*(\d+)')
    if (
        $resumedResponse.Content -notmatch 'event: task.completed' -or
        @($resumedIds | Where-Object { [int64]$_.Groups[1].Value -le $resumeAfter }).Count -ne 0
    ) {
        throw 'SSE Last-Event-ID resume returned duplicate or incomplete events'
    }

    $secondUserPassword = 'SecondUser123!'
    Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/users' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ username = 'second_user'; displayName = 'Second User'; role = 'user'; password = $secondUserPassword } | ConvertTo-Json) `
        -WebSession $session | Out-Null
    $secondLogin = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/auth/login' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ username = 'second_user'; password = $secondUserPassword } | ConvertTo-Json) `
        -SessionVariable secondSession
    if ($secondLogin.mustChangePassword) {
        throw 'Ordinary users must not be forced to change their initial password'
    }
    Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/providers' `
        -WebSession $secondSession | Out-Null
    $ordinaryPriceWrite = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}/pricing" -f $provider.id, $model.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ price = '1.00'; currency = 'USD' } | ConvertTo-Json) `
        -WebSession $secondSession `
        -SkipHttpErrorCheck
    if ($ordinaryPriceWrite.StatusCode -ne 403) {
        throw "Ordinary user pricing write expected 403, got $($ordinaryPriceWrite.StatusCode)"
    }
    Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/users/me/change-password' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ currentPassword = $secondUserPassword; newPassword = 'SecondChanged123!' } | ConvertTo-Json) `
        -WebSession $secondSession | Out-Null

    $adminSecurityBefore = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT json_build_object('role', role, 'status', status, 'passwordHash', password_hash, 'sessionVersion', session_version)::text FROM users WHERE id = '{0}'" -f $login.id)
    $userCountBefore = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        'SELECT COUNT(*) FROM users'
    $userAdminResponses = @(
        Invoke-WebRequest `
            -Uri 'http://127.0.0.1:3310/api/v1/admin/users' `
            -WebSession $secondSession `
            -SkipHttpErrorCheck
        Invoke-WebRequest `
            -Uri 'http://127.0.0.1:3310/api/v1/admin/users' `
            -Method Post `
            -ContentType 'application/json' `
            -Body (@{ username = 'rbac_forbidden_user'; displayName = 'Forbidden'; role = 'user'; password = 'Forbidden123!' } | ConvertTo-Json) `
            -WebSession $secondSession `
            -SkipHttpErrorCheck
        Invoke-WebRequest `
            -Uri ("http://127.0.0.1:3310/api/v1/admin/users/{0}" -f $login.id) `
            -Method Patch `
            -ContentType 'application/json' `
            -Body (@{ role = 'user'; status = 'disabled' } | ConvertTo-Json) `
            -WebSession $secondSession `
            -SkipHttpErrorCheck
        Invoke-WebRequest `
            -Uri ("http://127.0.0.1:3310/api/v1/admin/users/{0}/reset-password" -f $login.id) `
            -Method Post `
            -WebSession $secondSession `
            -SkipHttpErrorCheck
    )
    if (@($userAdminResponses | Where-Object StatusCode -ne 403).Count -ne 0) {
        throw 'A non-admin user accessed a user-management endpoint'
    }
    $adminSecurityAfter = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        ("SELECT json_build_object('role', role, 'status', status, 'passwordHash', password_hash, 'sessionVersion', session_version)::text FROM users WHERE id = '{0}'" -f $login.id)
    $userCountAfter = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        'SELECT COUNT(*) FROM users'
    if (
        (($adminSecurityBefore -join "`n").Trim() -ne ($adminSecurityAfter -join "`n").Trim()) -or
        (($userCountBefore -join "`n").Trim() -ne ($userCountAfter -join "`n").Trim())
    ) {
        throw 'Rejected user-management requests changed database state'
    }

    $crossUserUpload = New-TestUpload
    $crossUserDelete = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}" -f $crossUserUpload.id) `
        -Method Delete `
        -WebSession $secondSession `
        -SkipHttpErrorCheck
    $ownerCanStillRead = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}/content" -f $crossUserUpload.id) `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($crossUserDelete.StatusCode -ne 404 -or $ownerCanStillRead.StatusCode -ne 200) {
        throw 'Cross-user upload compensation delete bypassed Asset ownership'
    }
    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/image-assets/{0}" -f $crossUserUpload.id) `
        -Method Delete `
        -WebSession $session | Out-Null

    $crossUserChecks = @(
        @{ Uri = ("http://127.0.0.1:3310/api/v1/providers/{0}" -f $provider.id); Expected = 404 },
        @{ Uri = ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $second.Conversation.id); Expected = 404 },
        @{ Uri = ("http://127.0.0.1:3310/api/v1/tasks/{0}" -f $followUpTask.id); Expected = 404 },
        @{ Uri = ("http://127.0.0.1:3310/api/v1/image-assets/{0}/content" -f $followUpTask.results[0].id); Expected = 404 },
        @{ Uri = 'http://127.0.0.1:3310/api/v1/admin/storage'; Expected = 403 }
    )
    foreach ($check in $crossUserChecks) {
        $response = Invoke-WebRequest -Uri $check.Uri -WebSession $secondSession -SkipHttpErrorCheck
        if ($response.StatusCode -ne $check.Expected) {
            throw "Cross-user authorization check for $($check.Uri) expected $($check.Expected), got $($response.StatusCode)"
        }
    }
    $secondProviders = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/providers' `
        -WebSession $secondSession
    if (@($secondProviders).Count -ne 0) {
        throw 'Second user can see another user provider list'
    }

    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}" -f $second.Conversation.id) `
        -Method Delete `
        -WebSession $session | Out-Null
    $conversationList = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -WebSession $session
    if (@($conversationList | Where-Object id -eq $second.Conversation.id).Count -ne 0) {
        throw 'Conversation deletion did not remove the conversation'
    }
    $remainingFiles = @(Get-ChildItem -LiteralPath (Join-Path $testRoot 'images') -Recurse -File)
    if ($remainingFiles.Count -ne 0) {
        throw "Expected generated image files to be deleted, found $($remainingFiles.Count)"
    }

    $ownerId = [guid]::NewGuid().ToString()
    $orphanAssetId = [guid]::NewGuid().ToString()
    $orphanDirectory = Join-Path $testRoot "images\2020\01\$ownerId"
    $orphanPath = Join-Path $orphanDirectory "$orphanAssetId.png"
    New-Item -ItemType Directory -Path $orphanDirectory -Force | Out-Null
    [IO.File]::WriteAllBytes($orphanPath, [byte[]](1, 2, 3))
    (Get-Item -LiteralPath $orphanPath).LastWriteTimeUtc = (Get-Date).ToUniversalTime().AddMinutes(-5)

    $missingOwnerId = [guid]::NewGuid().ToString()
    $missingAssetId = [guid]::NewGuid().ToString()
    $missingKey = "2020/01/$missingOwnerId/$missingAssetId.png"
    $insertMissing = "INSERT INTO image_assets (owner_id, storage_driver, storage_container, storage_key, mime_type, width, height, file_size_bytes, sha256) SELECT id, 'local', 'default', '$missingKey', 'image/png', 1, 1, 3, repeat('0', 64) FROM users WHERE username = 'admin'"
    docker exec $containerName psql -U studio_test -d studio_test -v ON_ERROR_STOP=1 -c $insertMissing | Out-Null

    $scanOnly = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/storage/consistency/scan' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ deleteOrphans = $false } | ConvertTo-Json) `
        -WebSession $session
    if ($scanOnly.missingObjects -ne 1 -or $scanOnly.orphanObjects -ne 1 -or $scanOnly.deletedOrphans -ne 0) {
        throw 'Consistency scan did not report the expected missing/orphan objects'
    }
    $cleanupScan = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/storage/consistency/scan' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ deleteOrphans = $true } | ConvertTo-Json) `
        -WebSession $session
    if ($cleanupScan.deletedOrphans -ne 1 -or (Test-Path -LiteralPath $orphanPath)) {
        throw 'Consistency cleanup did not delete the eligible orphan object'
    }

    $partialConversation = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/conversations' `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = 'Native partial preview test'
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $partialCreated = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/conversations/{0}/messages" -f $partialConversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = 'Create a streamed preview image'
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1; partial_images = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    Wait-CompletedTask -TaskId $partialCreated.taskId | Out-Null
    $partialEvents = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/tasks/{0}/events" -f $partialCreated.taskId) `
        -Headers @{ Accept = 'text/event-stream' } `
        -WebSession $session
    $partialEventMatch = [regex]::Match(
        $partialEvents.Content,
        '(?ms)event:\s*image\.partial\r?\ndata:\s*(\{.*?\})\r?\n\r?\n'
    )
    if (-not $partialEventMatch.Success) {
        throw 'Native provider stream did not emit an image.partial application event'
    }
    $partialEventData = $partialEventMatch.Groups[1].Value | ConvertFrom-Json
    $partialPath = [string]$partialEventData.contentUrl
    $partialImage = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310{0}" -f $partialPath) `
        -WebSession $session
    if ($partialImage.StatusCode -ne 200 -or $partialImage.Headers.'Content-Type' -notmatch '^image/png') {
        throw 'Owned partial preview content was not available as an image'
    }
    $crossUserPartial = Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310{0}" -f $partialPath) `
        -WebSession $secondSession `
        -SkipHttpErrorCheck
    if ($crossUserPartial.StatusCode -ne 404) {
        throw "Cross-user partial preview access expected 404, got $($crossUserPartial.StatusCode)"
    }

    $seedUsage = "INSERT INTO usage_records (task_id, user_id, provider_id, model_id, quantity, unit, cost, currency, pricing_snapshot) SELECT NULL, u.id, '$($provider.id)', '$($model.id)', 1, 'image', 0.125, 'CNY', '{}'::jsonb FROM users u CROSS JOIN generate_series(1, 120) n WHERE u.username = 'admin'"
    docker exec $containerName psql -U studio_test -d studio_test -v ON_ERROR_STOP=1 -c $seedUsage | Out-Null
    $usagePage1 = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/usage?limit=50' `
        -WebSession $session
    if (@($usagePage1.recent).Count -ne 50 -or -not $usagePage1.nextBeforeId) {
        throw 'Usage first page did not expose 50 records and a cursor'
    }
    if ($usagePage1.totals.taskCount -lt 120 -or $usagePage1.totals.imageCount -lt 120) {
        throw 'Retained usage rows with deleted task IDs were not included in totals'
    }
    $cnyCost = @($usagePage1.costs | Where-Object currency -eq 'CNY')[0]
    if (-not $cnyCost -or $cnyCost.totalCost -lt 15) {
        throw 'Usage cost aggregation did not include retained CNY rows'
    }
    $usagePage2 = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/usage?limit=50&beforeId={0}" -f $usagePage1.nextBeforeId) `
        -WebSession $session
    if (@($usagePage2.recent).Count -ne 50) {
        throw 'Usage second page did not contain 50 records'
    }
    $usageIds1 = @($usagePage1.recent | ForEach-Object id)
    $usageIds2 = @($usagePage2.recent | ForEach-Object id)
    if (@($usageIds1 | Where-Object { $_ -in $usageIds2 }).Count -ne 0) {
        throw 'Usage cursor pagination returned duplicate records'
    }
    $invalidUsageLimit = Invoke-WebRequest `
        -Uri 'http://127.0.0.1:3310/api/v1/usage?limit=0' `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($invalidUsageLimit.StatusCode -ne 400) {
        throw "Invalid usage limit expected 400, got $($invalidUsageLimit.StatusCode)"
    }
    $secondUserUsage = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/usage' `
        -WebSession $secondSession
    if ($secondUserUsage.totals.taskCount -ne 0 -or $secondUserUsage.totals.imageCount -ne 0) {
        throw 'Usage aggregation leaked another user records'
    }

    $seedLogs = "INSERT INTO request_logs (trace_id, route, method, status_code, latency_ms) SELECT 'bulk-trace-' || n, '/bulk-test', 'GET', 200, n FROM generate_series(1, 120) n"
    docker exec $containerName psql -U studio_test -d studio_test -v ON_ERROR_STOP=1 -c $seedLogs | Out-Null
    $logPage1 = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/request-logs?limit=50' `
        -WebSession $session
    $logPage2 = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:3310/api/v1/admin/request-logs?limit=50&beforeId={0}" -f $logPage1.nextBeforeId) `
        -WebSession $session
    if (@($logPage1.items).Count -ne 50 -or @($logPage2.items).Count -ne 50) {
        throw 'Request log cursor pages did not contain 50 records each'
    }
    $logIds1 = @($logPage1.items | ForEach-Object id)
    $logIds2 = @($logPage2.items | ForEach-Object id)
    if (@($logIds1 | Where-Object { $_ -in $logIds2 }).Count -ne 0) {
        throw 'Request log cursor pagination returned duplicate records'
    }
    $traceResult = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/request-logs?traceId=bulk-trace-120' `
        -WebSession $session
    if (@($traceResult.items).Count -ne 1 -or $traceResult.items[0].traceId -ne 'bulk-trace-120') {
        throw 'Request log Trace ID filter returned an unexpected result'
    }
    $analytics = Invoke-RestMethod `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/analytics' `
        -WebSession $session
    if ($analytics.totals.totalTasks -lt 1 -or -not @($analytics.costs | Where-Object currency -eq 'CNY')) {
        throw 'Administrator analytics did not aggregate tasks and CNY costs'
    }
    $invalidPeriod = Invoke-WebRequest `
        -Uri 'http://127.0.0.1:3310/api/v1/admin/analytics?from=2026-01-02T00%3A00%3A00Z&to=2026-01-01T00%3A00%3A00Z' `
        -WebSession $session `
        -SkipHttpErrorCheck
    if ($invalidPeriod.StatusCode -ne 400) {
        throw "Invalid analytics period expected 400, got $($invalidPeriod.StatusCode)"
    }

    Invoke-WebRequest `
        -Uri ("http://127.0.0.1:3310/api/v1/providers/{0}/models/{1}/pricing/{2}" -f $provider.id, $model.id, $price.id) `
        -Method Delete `
        -WebSession $session | Out-Null
    $migrationCount = docker exec $containerName `
        psql -U studio_test -d studio_test -tAc `
        'SELECT COUNT(*) FROM _sqlx_migrations WHERE version IN (7, 8, 9, 10, 11)'
    if ($migrationCount.Trim() -ne '5') {
        throw 'Migrations 0007 through 0011 were not applied'
    }

    $rateLimited = $false
    for ($attempt = 0; $attempt -lt 280; $attempt++) {
        $response = Invoke-WebRequest `
            -Uri 'http://127.0.0.1:3310/api/v1/config' `
            -WebSession $session `
            -SkipHttpErrorCheck
        if ($response.StatusCode -eq 429) {
            if (-not $response.Headers['Retry-After']) {
                throw 'Rate limit response is missing Retry-After'
            }
            $rateLimited = $true
            break
        }
    }
    if (-not $rateLimited) {
        throw 'Rate limiter did not return HTTP 429'
    }

    Write-Output 'INTEGRATION_OK session_cookie_attributes=1 provider_credential_ciphertext=1 user_admin_rbac=1 host_updater_hmac=1 deployment_sync=1 provider_health=1 model_verify=1 model_test_generation=1 provider_rejection=1 pricing_admin_only=1 pricing_overlap=1 upload_compensation=1 referenced_asset_protection=1 provider_cancel=1 cancelled_message=1 manual_retry=1 multi_turn_context=1 image_edit=1 native_partial_preview=1 sse_resume=1 cross_user_isolation=1 cross_user_asset_delete=1 pricing_crud=1 history_filters=1 history_delete=1 conversation_delete=1 storage_cleanup=1 consistency_scan=1 orphan_grace_cleanup=1 usage_pagination=1 retained_usage=1 analytics_boundaries=1 request_log_pagination=1 migrations_0007_0008_0009_0010_0011=1 rate_limit=1'
}
finally {
    if ($appProcess -and -not $appProcess.HasExited) {
        Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
    }
    if ($mockProcess -and -not $mockProcess.HasExited) {
        Stop-Process -Id $mockProcess.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item Env:MOCK_UPDATER_TOKEN -ErrorAction SilentlyContinue
    if ($containerName.StartsWith('ai-image-studio-core-test-')) {
        docker rm -f $containerName 2>$null | Out-Null
    }
    $resolvedData = (Resolve-Path (Join-Path $workspace 'data')).Path
    if (
        (Test-Path -LiteralPath $testRoot) -and
        $testRoot.StartsWith($resolvedData + [IO.Path]::DirectorySeparatorChar) -and
        (Split-Path $testRoot -Leaf).StartsWith('integration-core-')
    ) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
