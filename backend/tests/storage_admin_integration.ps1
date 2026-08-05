$ErrorActionPreference = 'Stop'

$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$testRoot = Join-Path $workspace ("data\integration-storage-" + [guid]::NewGuid().ToString('N'))
$postgresContainer = "ai-image-studio-storage-pg-" + [guid]::NewGuid().ToString('N').Substring(0, 10)
$minioContainer = "ai-image-studio-storage-s3-" + [guid]::NewGuid().ToString('N').Substring(0, 10)
$bucket = "studio-test-" + [guid]::NewGuid().ToString('N').Substring(0, 12)
$s3Prefix = "mixed-assets-" + [guid]::NewGuid().ToString('N').Substring(0, 12)
$appPort = 3312
$mockPort = 3403
$postgresPort = 55435
$minioPort = 59000
$binaryName = if ($IsWindows) { 'ai-image-studio.exe' } else { 'ai-image-studio' }
$binaryPath = [IO.Path]::Combine($workspace, 'target', 'debug', $binaryName)
$mockProcess = $null
$appProcess = $null
$appSequence = 0

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

function Start-TestApp {
    $script:appSequence += 1
    Start-HiddenProcess `
        -FilePath $binaryPath `
        -ArgumentList @('serve') `
        -WorkingDirectory $workspace `
        -StandardOutput (Join-Path $testRoot "app-$appSequence.out.log") `
        -StandardError (Join-Path $testRoot "app-$appSequence.err.log")
}

function Wait-AppReady {
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$appPort/api/v1/health"
            if ($health.status -eq 'ok') {
                return
            }
        }
        catch {
            # Migrations and bootstrap can still be running.
        }
        Start-Sleep -Milliseconds 500
    }
    throw 'Application did not become ready'
}

function Wait-CompletedTask {
    param([string] $TaskId)

    for ($attempt = 0; $attempt -lt 60; $attempt++) {
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

function New-CompletedTask {
    param([string] $Title)

    $conversation = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/conversations" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            title = $Title
            defaultProviderId = $provider.id
            defaultModelId = $model.id
        } | ConvertTo-Json) `
        -WebSession $session
    $created = Invoke-RestMethod `
        -Uri ("http://127.0.0.1:{0}/api/v1/conversations/{1}/messages" -f $appPort, $conversation.id) `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            content = "Create $Title"
            providerId = $provider.id
            modelId = $model.id
            stream = $false
            parameters = @{ size = 'auto'; n = 1 }
            inputAssetIds = @()
        } | ConvertTo-Json -Depth 5) `
        -WebSession $session
    Wait-CompletedTask -TaskId $created.taskId
}

$appPort = Get-FreePort
$mockPort = Get-FreePort
$postgresPort = Get-FreePort
$minioPort = Get-FreePort
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
        -e POSTGRES_DB=studio_storage_test `
        -e POSTGRES_USER=studio_storage_test `
        -e POSTGRES_PASSWORD=studio_storage_test `
        -p ("127.0.0.1:{0}:5432" -f $postgresPort) `
        -d postgres:17-alpine | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to start PostgreSQL container'
    }
    docker run --name $minioContainer `
        -e MINIO_ROOT_USER=minioadmin `
        -e MINIO_ROOT_PASSWORD=minioadmin123 `
        -p ("127.0.0.1:{0}:9000" -f $minioPort) `
        -d minio/minio:RELEASE.2024-10-13T13-34-11Z server /data | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to start MinIO container'
    }

    $postgresReady = $false
    $minioReady = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if (-not $postgresReady) {
            docker exec $postgresContainer pg_isready -U studio_storage_test -d studio_storage_test 2>$null | Out-Null
            $postgresReady = $LASTEXITCODE -eq 0
        }
        if (-not $minioReady) {
            try {
                $minioReady = (Invoke-WebRequest -Uri "http://127.0.0.1:$minioPort/minio/health/ready").StatusCode -eq 200
            }
            catch {
                $minioReady = $false
            }
        }
        if ($postgresReady -and $minioReady) {
            break
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $postgresReady -or -not $minioReady) {
        throw 'PostgreSQL or MinIO did not become ready'
    }

    docker run --rm --network ("container:{0}" -f $minioContainer) `
        --entrypoint /bin/sh minio/mc:RELEASE.2024-10-08T09-37-26Z `
        -c "mc alias set integration http://127.0.0.1:9000 minioadmin minioadmin123 >/dev/null && mc mb integration/$bucket >/dev/null" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to create MinIO test bucket'
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
    $env:DATABASE_URL = "postgres://studio_storage_test:studio_storage_test@127.0.0.1:$postgresPort/studio_storage_test"
    $env:SESSION_SECRET = 'storage-integration-session-secret-at-least-32-characters'
    $env:CREDENTIAL_MASTER_KEY = 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA='
    $env:STORAGE_DRIVER = 'local'
    $env:STORAGE_LOCAL_PATH = Join-Path $testRoot 'images'
    $env:STORAGE_S3_ENABLED = 'true'
    $env:STORAGE_S3_BUCKET = $bucket
    $env:STORAGE_S3_REGION = 'us-east-1'
    $env:STORAGE_S3_ENDPOINT = "http://127.0.0.1:$minioPort"
    $env:STORAGE_S3_PREFIX = $s3Prefix
    $env:STORAGE_S3_ACCESS_KEY_ID = 'minioadmin'
    $env:STORAGE_S3_SECRET_ACCESS_KEY = 'minioadmin123'
    $env:STORAGE_S3_FORCE_PATH_STYLE = 'true'
    $env:STORAGE_CONSISTENCY_SCAN_ENABLED = 'false'
    $env:TASK_EXECUTION_MODE = 'inline'
    $env:REDIS_URL = ''
    $env:TASK_MAX_RETRIES = '0'
    $env:RATE_LIMIT_ENABLED = 'false'
$env:ALLOW_CUSTOM_BASE_URL = 'true'
$env:ALLOW_PRIVATE_PROVIDER_HOSTS = 'true'
    $env:BOOTSTRAP_ADMIN_ENABLED = 'true'
    $env:BOOTSTRAP_ADMIN_USERNAME = 'admin'
    $env:BOOTSTRAP_ADMIN_PASSWORD = '123456'
    $env:BOOTSTRAP_ADMIN_FORCE_PASSWORD_CHANGE = 'true'
    $env:RUST_LOG = 'ai_image_studio=warn'

    $appProcess = Start-TestApp
    Wait-AppReady
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
        -Body (@{ currentPassword = '123456'; newPassword = 'StorageIntegration123!' } | ConvertTo-Json) `
        -WebSession $session | Out-Null

    $provider = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/providers" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{
            providerKey = 'storage-mock'
            providerType = 'openai-compatible'
            displayName = 'Storage Mock'
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

    $localTask = New-CompletedTask -Title 'Local asset before switch'
    if (@($localTask.results).Count -ne 1) {
        throw 'Local task did not produce one image'
    }

    $target = @{
        driver = 's3'
        localPath = (Join-Path $testRoot 'images')
        s3Bucket = $bucket
        s3Region = 'us-east-1'
        s3Endpoint = "http://127.0.0.1:$minioPort"
        s3Prefix = $s3Prefix
        s3ForcePathStyle = $true
    }
    $testResult = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/admin/storage/test" `
        -Method Post `
        -ContentType 'application/json' `
        -Body ($target | ConvertTo-Json) `
        -WebSession $session
    if (-not $testResult.ok -or $testResult.driver -ne 's3') {
        throw 'Administrator S3 round-trip test failed'
    }
    $saveResult = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/admin/storage" `
        -Method Put `
        -ContentType 'application/json' `
        -Body ($target | ConvertTo-Json) `
        -WebSession $session
    if (-not $saveResult.restartRequired) {
        throw 'Storage update did not require a restart'
    }
    $beforeRestart = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/admin/storage" `
        -WebSession $session
    if ($beforeRestart.activeDriver -ne 'local' -or $beforeRestart.targetConfig.driver -ne 's3') {
        throw 'Runtime and target storage drivers were not reported separately'
    }

    Stop-Process -Id $appProcess.Id -Force
    Wait-Process -Id $appProcess.Id -ErrorAction SilentlyContinue
    $appProcess = Start-TestApp
    Wait-AppReady
    Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/auth/login" `
        -Method Post `
        -ContentType 'application/json' `
        -Body (@{ username = 'admin'; password = 'StorageIntegration123!' } | ConvertTo-Json) `
        -SessionVariable session | Out-Null

    $afterRestart = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/admin/storage" `
        -WebSession $session
    if ($afterRestart.activeDriver -ne 's3' -or -not $afterRestart.s3Configured) {
        throw 'Persisted S3 target did not become active after restart'
    }
    $serializedStorage = $afterRestart | ConvertTo-Json -Depth 10
    if ($serializedStorage -match 'minioadmin123' -or $serializedStorage -match 'accessKey|secretKey') {
        throw 'Storage configuration response leaked S3 credentials'
    }

    $s3Task = New-CompletedTask -Title 'S3 asset after switch'
    if (@($s3Task.results).Count -ne 1) {
        throw 'S3 task did not produce one image'
    }
    $mixed = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/admin/storage" `
        -WebSession $session
    if ($mixed.localAssetCount -ne 1 -or $mixed.s3AssetCount -ne 1) {
        throw "Expected one Local and one S3 asset, got $($mixed.localAssetCount)/$($mixed.s3AssetCount)"
    }
    $historyResponse = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$appPort/api/v1/history" `
        -WebSession $session
    $history = @($historyResponse)
    if ($history.Count -ne 2) {
        throw "Expected two mixed-storage history results, got $($history.Count)"
    }
    foreach ($item in $history) {
        $image = Invoke-WebRequest `
            -Uri ("http://127.0.0.1:{0}{1}" -f $appPort, $item.contentUrl) `
            -WebSession $session
        if ($image.StatusCode -ne 200 -or $image.Headers.'Content-Type' -notmatch '^image/png') {
            throw "Mixed-storage image $($item.assetId) is unavailable"
        }
    }

    $objects = @(docker run --rm --network ("container:{0}" -f $minioContainer) `
            --entrypoint /bin/sh minio/mc:RELEASE.2024-10-08T09-37-26Z `
            -c "mc alias set integration http://127.0.0.1:9000 minioadmin minioadmin123 >/dev/null && mc ls --recursive --json integration/$bucket/$s3Prefix")
    $objectEntries = @($objects | Where-Object { $_.Trim() } | ForEach-Object {
            $_ | ConvertFrom-Json
        })
    $objectKeys = @($objectEntries | ForEach-Object { $_.key })
    $healthCheckObjects = @($objectKeys | Where-Object { $_ -like 'health-check/*' })
    if ($healthCheckObjects.Count -ne 0) {
        throw "Storage health check left temporary objects: $($healthCheckObjects -join ', ')"
    }
    if ($objectEntries.Count -ne 2) {
        throw "Expected one durable S3 image and one live partial preview, got $($objectEntries.Count) ($($objectKeys -join ', '))"
    }

    Write-Output 'STORAGE_ADMIN_INTEGRATION_OK s3_round_trip=1 persisted_switch=1 no_secret_echo=1 local_s3_counts=1 mixed_reads=1 health_object_cleanup=1 partial_preview_ttl=1'
}
finally {
    foreach ($process in @($appProcess, $mockProcess)) {
        if ($process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if ($postgresContainer.StartsWith('ai-image-studio-storage-pg-')) {
        docker rm -f $postgresContainer 2>$null | Out-Null
    }
    if ($minioContainer.StartsWith('ai-image-studio-storage-s3-')) {
        docker rm -f $minioContainer 2>$null | Out-Null
    }
    $resolvedData = (Resolve-Path (Join-Path $workspace 'data')).Path
    if (
        (Test-Path -LiteralPath $testRoot) -and
        $testRoot.StartsWith($resolvedData + [IO.Path]::DirectorySeparatorChar) -and
        (Split-Path $testRoot -Leaf).StartsWith('integration-storage-')
    ) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
