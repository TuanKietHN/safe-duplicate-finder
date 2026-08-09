[CmdletBinding()]
param(
    [string]$CliPath = "target\debug\safe-dedupe.exe"
)

$ErrorActionPreference = "Stop"
$temporaryRoot = (Resolve-Path -LiteralPath $env:TEMP).Path
$runRoot = Join-Path $temporaryRoot ("safe-dedupe-quickstart-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $runRoot | Out-Null

try {
    $source = Join-Path $runRoot "source"
    New-Item -ItemType Directory -Path (Join-Path $source "primary") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $source "copy") -Force | Out-Null
    $payload = [Text.Encoding]::UTF8.GetBytes("quickstart-identical-payload")
    [IO.File]::WriteAllBytes((Join-Path $source "primary\book.pdf"), $payload)
    [IO.File]::WriteAllBytes((Join-Path $source "copy\book.pdf"), $payload)

    $database = Join-Path $runRoot "state.db"
    $cli = (Resolve-Path -LiteralPath $CliPath).Path
    $created = & $cli --database $database project create --name "Quickstart validation"
    if ($LASTEXITCODE -ne 0) { throw "project create failed" }
    $project = [regex]::Match(($created -join "`n"), "[0-9a-f]{8}-[0-9a-f-]{27}").Value

    & $cli --database $database project add-root --project $project --path $source --primary
    if ($LASTEXITCODE -ne 0) { throw "add root failed" }
    $scan = (& $cli --database $database scan start --project $project --all-files) |
        ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "scan failed" }
    $planOutput = & $cli --database $database plan create --session $scan.session_id --policy default
    if ($LASTEXITCODE -ne 0) { throw "plan failed" }
    $plan = [regex]::Match(($planOutput -join "`n"), "[0-9a-f]{8}-[0-9a-f-]{27}").Value
    $dryRun = (& $cli --database $database dry-run --plan $plan --json) | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "dry-run failed" }

    & $cli --database $database quarantine apply --plan $plan --confirm QUARANTINE
    if ($LASTEXITCODE -ne 0) { throw "quarantine failed" }
    $inventory = @(
        (& $cli --database $database quarantine list --project $project --json) |
            ConvertFrom-Json
    )
    if ($LASTEXITCODE -ne 0) { throw "inventory failed" }
    if ($inventory.Count -ne 1 -or $inventory[0].state -ne "verified") {
        throw "verified quarantine inventory mismatch"
    }

    & $cli --database $database restore --entry $inventory[0].id --confirm RESTORE
    if ($LASTEXITCODE -ne 0) { throw "restore failed" }
    $first = [IO.File]::ReadAllBytes((Join-Path $source "primary\book.pdf"))
    $second = [IO.File]::ReadAllBytes((Join-Path $source "copy\book.pdf"))
    if (-not [Linq.Enumerable]::SequenceEqual($first, $second)) {
        throw "restored content mismatch"
    }

    [pscustomobject]@{
        Project = $project
        Session = $scan.session_id
        Groups = $scan.duplicate_groups
        Plan = $plan
        DryRunFiles = $dryRun.quarantine_files
        DryRunBytes = $dryRun.quarantine_bytes
        QuarantineEntries = $inventory.Count
        RestoreMatched = $true
    }
}
finally {
    $absolute = [IO.Path]::GetFullPath($runRoot)
    $prefix = $temporaryRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) +
        [IO.Path]::DirectorySeparatorChar
    if ($absolute.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $absolute)) {
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
}
