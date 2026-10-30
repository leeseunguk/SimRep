<#
.SYNOPSIS
  Stages the OpenCascade runtime closure (and its license texts) for the Windows installer.

.DESCRIPTION
  Starting from the 20 OCCT toolkits the app delay-loads (app/src-tauri/build.rs), follows every
  DLL import with `dumpbin /DEPENDENTS` through the same folders OCCT's env.bat/custom.bat put on
  PATH (<CASROOT>\win64\vc14\bin plus each bundled third-party <name>\bin and openvr bin\win64),
  and copies the closure flat into -Dest. Imports not found there (Windows system DLLs, the
  UCRT api-sets, the MSVC C++ runtime) are reported and not copied. The MSVC C++ runtime DLLs the
  closure needs (msvcp140*.dll, vcruntime140*.dll, concrt140.dll, and what those import in turn)
  are copied from the Visual Studio redistributable folder into -Dest\crt: the installer puts them
  next to SimRep.exe, which imports MSVCP140.dll itself (the simrep-cad C++ shim), so the app starts
  and imports CAD files where the VC++ Redistributable is not installed. The OCCT DLLs, loaded
  later, reuse those already-loaded modules.

  Also writes the LGPL notices: OCCT's LICENSE_LGPL_21.txt and OCCT_LGPL_EXCEPTION.txt, the
  license of every bundled third-party library, and THIRD-PARTY-NOTICES.txt listing each DLL.

  The tauri.cad.conf.json overlay bundles -Dest as the `occt\` folder next to SimRep's exe,
  where src-tauri/src/cad.rs looks for it, and -Dest\crt into the install folder itself.
  tauri.windows.conf.json moves the OCCT DLLs beside both executables for the CLI's startup imports.

.EXAMPLE
  powershell -NoProfile -ExecutionPolicy Bypass -File tools\win\stage-occt.ps1 -Dest app\src-tauri\bundle-occt
#>
param(
  [Parameter(Mandatory = $true)][string]$Dest,
  [string]$CasRoot = (Join-Path $env:USERPROFILE 'occt\opencascade-7.7.0'),
  [string]$Dumpbin = ''
)
$ErrorActionPreference = 'Stop'

$toolkits = @('TKXDESTEP', 'TKXDEIGES', 'TKSTEP', 'TKIGES', 'TKXSBase', 'TKXCAF', 'TKCAF', 'TKLCAF', 'TKCDF', 'TKMesh', 'TKShHealing', 'TKPrim',
  'TKTopAlgo', 'TKGeomAlgo', 'TKBRep', 'TKGeomBase', 'TKG3d', 'TKG2d', 'TKMath', 'TKernel')

$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath
if (-not $Dumpbin) {
  $Dumpbin = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') | Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
}
$crt = Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT') -Directory | Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName

# env.bat / custom.bat search folders.
$products = Split-Path $CasRoot -Parent
$dirs = @(Join-Path $CasRoot 'win64\vc14\bin')
foreach ($p in Get-ChildItem $products -Directory) {
  foreach ($sub in @('bin', 'bin\win64')) {
    $d = Join-Path $p.FullName $sub
    if ((Test-Path $d) -and $d -ne $dirs[0]) { $dirs += $d }
  }
}

function Find-Dll([string]$name) {
  foreach ($d in $dirs) {
    $f = Join-Path $d $name
    if (Test-Path $f) { return (Get-Item $f).FullName }
  }
  return $null
}

$found = [ordered]@{}   # lower-case name -> full path
$external = @{}         # lower-case name -> original name
$queue = New-Object System.Collections.Queue
foreach ($t in $toolkits) { $queue.Enqueue("$t.dll") }
while ($queue.Count -gt 0) {
  $name = $queue.Dequeue()
  $key = $name.ToLowerInvariant()
  if ($found.Contains($key) -or $external.ContainsKey($key)) { continue }
  $path = Find-Dll $name
  if (-not $path) { $external[$key] = $name; continue }
  $found[$key] = $path
  $inList = $false
  foreach ($line in & $Dumpbin /NOLOGO /DEPENDENTS $path) {
    if ($line -match 'Image has the following (delay load )?dependencies') { $inList = $true; continue }
    if ($inList -and $line -match '^\s+(\S+\.dll)\s*$') { $queue.Enqueue($Matches[1]) }
    elseif ($inList -and $line -match '^\s+Summary') { break }
  }
}
foreach ($t in $toolkits) {
  if (-not $found.Contains("$t.dll".ToLowerInvariant())) { throw "toolkit $t.dll not found under $($dirs -join '; ')" }
}

if (Test-Path $Dest) { Remove-Item $Dest -Recurse -Force }
New-Item -ItemType Directory $Dest | Out-Null
New-Item -ItemType Directory (Join-Path $Dest 'licenses') | Out-Null

$bytes = 0
$rows = @()
foreach ($entry in $found.GetEnumerator()) {
  Copy-Item $entry.Value $Dest
  $bytes += (Get-Item $entry.Value).Length
  $product = (Split-Path (Split-Path $entry.Value -Parent) -Parent)
  if ($product -like '*\win64\vc14') { $product = $CasRoot } elseif ((Split-Path $product -Leaf) -eq 'bin') { $product = Split-Path $product -Parent }
  $rows += [pscustomobject]@{ Dll = (Split-Path $entry.Value -Leaf); Product = (Split-Path $product -Leaf); Source = $entry.Value }
}

# App-local MSVC C++ runtime: the closure's runtime imports and, recursively, theirs
# (msvcp140.dll imports vcruntime140.dll and vcruntime140_1.dll).
New-Item -ItemType Directory (Join-Path $Dest 'crt') | Out-Null
$crtCopied = @()
$crtQueue = New-Object System.Collections.Queue
foreach ($key in @($external.Keys)) { $crtQueue.Enqueue($external[$key]) }
while ($crtQueue.Count -gt 0) {
  $name = $crtQueue.Dequeue()
  $f = Join-Path $crt $name
  if (-not (Test-Path $f) -or ($crtCopied -contains $name)) { continue }
  Copy-Item $f (Join-Path $Dest 'crt')
  $bytes += (Get-Item $f).Length
  $crtCopied += $name
  $external.Remove($name.ToLowerInvariant())
  $inList = $false
  foreach ($line in & $Dumpbin /NOLOGO /DEPENDENTS $f) {
    if ($line -match 'Image has the following (delay load )?dependencies') { $inList = $true; continue }
    if ($inList -and $line -match '^\s+(\S+\.dll)\s*$') { $crtQueue.Enqueue($Matches[1]) }
    elseif ($inList -and $line -match '^\s+Summary') { break }
  }
}

# License texts.
Copy-Item (Join-Path $CasRoot 'LICENSE_LGPL_21.txt') (Join-Path $Dest 'licenses\OCCT-LICENSE_LGPL_21.txt')
Copy-Item (Join-Path $CasRoot 'OCCT_LGPL_EXCEPTION.txt') (Join-Path $Dest 'licenses\OCCT_LGPL_EXCEPTION.txt')
$thirdParty = $rows | Where-Object { $_.Product -ne (Split-Path $CasRoot -Leaf) } | Select-Object -ExpandProperty Product -Unique
foreach ($p in $thirdParty) {
  $root = Join-Path $products $p
  $licenses = Get-ChildItem $root -Recurse -File -Include 'LICENSE*', 'COPYING*', 'license*.txt', '*LICENSE*.txt' -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\include\\|\\test' }
  foreach ($l in $licenses) { Copy-Item $l.FullName (Join-Path $Dest "licenses\$p-$($l.Name)") }
  # The OCCT 7.7.0 archive ships no license text for tbb, freetype, freeimage and ffmpeg; the
  # upstream texts of those versions are kept in tools\win\occt-licenses (see its README.md).
  $short = ($p -split '[-_]')[0]
  foreach ($l in Get-ChildItem (Join-Path $PSScriptRoot 'occt-licenses') -File -Filter "$short-*") { Copy-Item $l.FullName (Join-Path $Dest "licenses\$($l.Name)") }
  if ($short -eq 'ffmpeg') { Copy-Item (Join-Path $CasRoot 'LICENSE_LGPL_21.txt') (Join-Path $Dest 'licenses\ffmpeg-COPYING.LGPLv2.1.txt') }
}

# Notice file.
$occtVersion = (Split-Path $CasRoot -Leaf)
$lines = @(
  'SimRep - third-party OCCT runtime notices',
  '========================================',
  '',
  'SimRep imports STEP and IGES files through Open CASCADE Technology (OCCT).',
  "The bundled DLLs are the unmodified official $occtVersion Windows x64 (vc14) binaries",
  'from https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V7_7_0 and the third-party',
  'libraries shipped with that release. CAD-only bundles put the DLLs in occt\;',
  'the self-contained Windows bundle shares them beside simrep-app.exe and simrep.exe.',
  '',
  'Open CASCADE Technology is licensed under the GNU Lesser General Public License',
  'version 2.1 with the Open CASCADE exception (licenses\OCCT-LICENSE_LGPL_21.txt,',
  'licenses\OCCT_LGPL_EXCEPTION.txt). SimRep links to OCCT dynamically: simrep-app.exe',
  'delay-loads these DLLs at run time; simrep.exe has ordinary dynamic startup imports.',
  'You may replace the DLLs in their installed directory with your own build of the',
  'same OCCT version (same DLL names and exported ABI). The app also accepts an',
  'alternative via SIMREP_OCCT_DIR (an OCCT CASROOT or a folder holding the DLLs).',
  'OCCT source code is',
  'available from https://dev.opencascade.org and https://github.com/Open-Cascade-SAS/OCCT',
  '(tag V7_7_0); on request we provide the exact source of the bundled version.',
  ''
)
if ($thirdParty) {
  $lines += 'Third-party libraries used by these OCCT binaries (licenses in licenses\):'
  foreach ($p in $thirdParty) {
    $dlls = ($rows | Where-Object { $_.Product -eq $p } | ForEach-Object { $_.Dll }) -join ', '
    $lines += "  - ${p}: $dlls"
  }
  $lines += ''
}
if ($crtCopied) {
  $lines += 'Microsoft Visual C++ runtime, installed next to SimRep.exe (app-local copy of the Visual'
  $lines += "Studio redistributable files, distributable under the Visual Studio license terms): $($crtCopied -join ', ')"
  $lines += ''
}
$lines += 'Files:'
foreach ($r in $rows) { $lines += "  $($r.Dll)  ($($r.Product))" }
$lines | Set-Content -Encoding UTF8 (Join-Path $Dest 'THIRD-PARTY-NOTICES.txt')

Write-Output "dumpbin: $Dumpbin"
Write-Output "search folders: $($dirs -join '; ')"
Write-Output ("closure: {0} DLL(s) from the OCCT folders, {1} MSVC runtime DLL(s): {2:N1} MB" -f $found.Count, $crtCopied.Count, ($bytes / 1MB))
foreach ($r in $rows) { Write-Output ("  {0,-28} {1}" -f $r.Dll, $r.Product) }
Write-Output "MSVC runtime copied to crt\: $($crtCopied -join ', ')"
Write-Output "system imports (not bundled): $((($external.Values) | Sort-Object) -join ', ')"
Write-Output "licenses: $((Get-ChildItem (Join-Path $Dest 'licenses') | ForEach-Object Name) -join ', ')"
