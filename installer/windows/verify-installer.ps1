[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateScript({ Test-Path -LiteralPath $_ -PathType Leaf })]
    [string]$InstallerPath
)

$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path -LiteralPath $InstallerPath).Path
$item = Get-Item -LiteralPath $resolved
if ($item.Length -ge 15MB) {
    throw "Online setup vượt 15 MiB: $($item.Length) byte"
}
if ($item.Extension -ne '.exe') {
    throw 'Artifact phát hành phải là tệp .exe'
}

$hash = Get-FileHash -Algorithm SHA256 -LiteralPath $resolved
$signature = Get-AuthenticodeSignature -LiteralPath $resolved

[pscustomobject]@{
    Path = $resolved
    SizeBytes = $item.Length
    SHA256 = $hash.Hash
    AuthenticodeStatus = $signature.Status
    ProductVersion = $item.VersionInfo.ProductVersion
} | Format-List

if ($signature.Status -notin @('Valid', 'NotSigned')) {
    throw "Chữ ký Authenticode có trạng thái không an toàn: $($signature.Status)"
}

Write-Host 'Kiểm tra tĩnh online setup đạt. Tiếp tục qualification trên máy sạch theo quickstart.md.'
