param(
    [string]$Image = 'safe-dedupe:smoke',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$docker = Get-Command docker -ErrorAction Stop

if (-not $SkipBuild) {
    & $docker.Source build --tag $Image $workspace
    if ($LASTEXITCODE -ne 0) { throw 'Docker image build failed.' }
}

$configuredUser = & $docker.Source image inspect $Image --format '{{.Config.User}}'
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect Docker image.' }
if ($configuredUser.Trim() -ne 'safe-dedupe') {
    throw "Container must run as safe-dedupe, found '$configuredUser'."
}

$temporary = Join-Path ([IO.Path]::GetTempPath()) ("safe-dedupe-container-{0}" -f [guid]::NewGuid())
$source = Join-Path $temporary 'source'
$data = Join-Path $temporary 'data'
$reports = Join-Path $temporary 'reports'
New-Item -ItemType Directory -Path $source, $data, $reports | Out-Null
try {
    [IO.File]::WriteAllBytes((Join-Path $source 'source.pdf'), [Text.Encoding]::UTF8.GetBytes('source-must-remain'))

    & $docker.Source run --rm `
        --volume "${source}:/scan:ro" `
        --volume "${data}:/data:rw" `
        --volume "${reports}:/reports:rw" `
        --env SAFE_DEDUPE_MODE=scan `
        $Image check
    if ($LASTEXITCODE -ne 0) { throw 'Read-only non-root scan-mode smoke failed.' }

    & $docker.Source run --rm `
        --volume "${source}:/scan:rw" `
        --volume "${data}:/data:rw" `
        --env SAFE_DEDUPE_MODE=scan `
        $Image check
    if ($LASTEXITCODE -ne 3) { throw 'Scan mode did not reject a read-write source mount.' }

    & $docker.Source run --rm `
        --volume "${source}:/scan:ro" `
        --volume "${data}:/data:rw" `
        --env SAFE_DEDUPE_MODE=quarantine `
        --env SAFE_DEDUPE_QUARANTINE_ROOT=/scan/.safe-duplicate-finder-quarantine `
        $Image check
    if ($LASTEXITCODE -ne 3) { throw 'Quarantine mode did not reject a read-only source mount.' }

    $content = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes((Join-Path $source 'source.pdf')))
    if ($content -ne 'source-must-remain') { throw 'Container smoke changed source content.' }
}
finally {
    $resolvedTemporary = [IO.Path]::GetFullPath($temporary)
    $resolvedTempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (-not $resolvedTemporary.StartsWith($resolvedTempRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove unexpected smoke-test path: $resolvedTemporary"
    }
    Remove-Item -LiteralPath $resolvedTemporary -Recurse -Force
}
