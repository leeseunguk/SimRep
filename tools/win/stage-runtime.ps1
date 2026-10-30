<# Stages the pinned, portable Node runtime and CLI, and verifies complete installer notices. Stage OCCT first. #>
param(
  [Parameter(Mandatory = $true)][string]$Dest,
  [Parameter(Mandatory = $true)][string]$Cli,
  [string]$Archive = ''
)
$ErrorActionPreference = 'Stop'
$version = '24.14.1'
$sha256 = '6e50ce5498c0cebc20fd39ab3ff5df836ed2f8a31aa093cecad8497cff126d70'
if (-not $Archive) {
  $Archive = Join-Path $env:TEMP "node-v$version-win-x64.zip"
  Invoke-WebRequest "https://nodejs.org/dist/v$version/node-v$version-win-x64.zip" -OutFile $Archive -UseBasicParsing -TimeoutSec 120
}
if ((Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sha256) {
  throw 'Pinned Node archive SHA-256 mismatch'
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
New-Item -ItemType Directory -Force $Dest | Out-Null
$zip = [IO.Compression.ZipFile]::OpenRead($Archive)
try {
  foreach ($name in @('node.exe', 'LICENSE')) {
    $entry = $zip.GetEntry("node-v$version-win-x64/$name")
    if (-not $entry) { throw "Node archive is missing $name" }
    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $Dest $name), $true)
  }
} finally { $zip.Dispose() }
Copy-Item $Cli (Join-Path $Dest 'simrep.exe') -Force
& (Join-Path $PSScriptRoot 'stage-notices.ps1')
Get-ChildItem $Dest | Select-Object Name, Length
Get-FileHash (Join-Path $Dest 'node.exe'), (Join-Path $Dest 'simrep.exe') -Algorithm SHA256
