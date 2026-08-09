[CmdletBinding()]
param(
    [switch]$SkipTests
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$desktopRoot = Join-Path $projectRoot 'apps\desktop'
$outputRoot = Join-Path $projectRoot 'target\release\online-installer'
$version = '0.2.1'

function Invoke-Checked {
    param(
        [Parameter(Mandatory)] [string]$Program,
        [Parameter(ValueFromRemainingArguments)] [string[]]$Arguments
    )
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Lệnh thất bại ($LASTEXITCODE): $Program $($Arguments -join ' ')"
    }
}

Push-Location $projectRoot
try {
    if (-not $SkipTests) {
        Invoke-Checked -Program 'cargo' -Arguments @('fmt', '--all', '--', '--check')
        Invoke-Checked -Program 'cargo' -Arguments @('test', '-p', 'safe-dedupe-runtime-installer', '--all-targets')
        Invoke-Checked -Program 'cargo' -Arguments @('clippy', '-p', 'safe-dedupe-runtime-installer', '--all-targets', '--', '-D', 'warnings')
    }

    # Helper phải tự chạy được trước khi bất kỳ Runtime bổ sung nào tồn tại.
    $savedRustFlags = $env:RUSTFLAGS
    try {
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        Invoke-Checked -Program 'cargo' -Arguments @('build', '-p', 'safe-dedupe-runtime-installer', '--release')
    }
    finally {
        $env:RUSTFLAGS = $savedRustFlags
    }

    $helper = Join-Path $projectRoot 'target\release\safe-dedupe-runtime-installer.exe'
    if (-not (Test-Path -LiteralPath $helper)) {
        throw "Không tìm thấy helper Runtime: $helper"
    }

    Invoke-Checked -Program 'npm' -Arguments @(
        '--prefix', $desktopRoot,
        'run', 'tauri', '--',
        'build', '--bundles', 'nsis',
        '--config', 'src-tauri/tauri.online-installer.conf.json'
    )

    $bundleRoot = Join-Path $projectRoot 'target\release\bundle\nsis'
    $payload = Get-ChildItem -LiteralPath $bundleRoot -File |
        Where-Object { $_.Name -like "*_${version}_x64-setup.exe" } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $payload) {
        throw "Không tìm thấy NSIS setup phiên bản $version trong $bundleRoot"
    }

    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    $finalInstaller = Join-Path $outputRoot "safe-dedupe-setup-${version}-x64.exe"
    Copy-Item -LiteralPath $payload.FullName -Destination $finalInstaller -Force

    $installerItem = Get-Item -LiteralPath $finalInstaller
    if ($installerItem.Length -ge 15MB) {
        throw "Online setup vượt ngân sách 15 MiB: $($installerItem.Length) byte"
    }
    $installerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $finalInstaller).Hash
    $helperHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $helper).Hash
    $manifestPath = Join-Path $projectRoot 'installer\runtime-manifest.json'
    $manifestHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $manifestPath).Hash
    $application = Join-Path $projectRoot 'target\release\safe-dedupe-desktop.exe'
    if (-not (Test-Path -LiteralPath $application -PathType Leaf)) {
        throw "Không tìm thấy ứng dụng release: $application"
    }
    $applicationItem = Get-Item -LiteralPath $application
    $applicationHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $application).Hash
    $summary = [ordered]@{
        schema_version = 1
        release_version = $version
        created_at = (Get-Date).ToUniversalTime().ToString('o')
        installer = [ordered]@{
            path = $finalInstaller
            size_bytes = $installerItem.Length
            sha256 = $installerHash
        }
        runtime_helper = [ordered]@{
            path = $helper
            size_bytes = (Get-Item -LiteralPath $helper).Length
            sha256 = $helperHash
            crt_static = $true
        }
        application = [ordered]@{
            path = $application
            size_bytes = $applicationItem.Length
            sha256 = $applicationHash
        }
        runtime_manifest_sha256 = $manifestHash
    }
    $summaryPath = Join-Path $outputRoot 'release-checksums.json'
    $summary | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $summaryPath -Encoding utf8

    Write-Host "Đã tạo online setup: $finalInstaller"
    Write-Host "Kích thước: $($installerItem.Length) byte"
    Write-Host "SHA-256: $installerHash"
    Write-Host "Bằng chứng: $summaryPath"
}
finally {
    Pop-Location
}
