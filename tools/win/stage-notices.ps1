<# Map every staged native DLL to full notices, then generate the installer-wide inventory. #>
param()
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$occt = Join-Path $root 'app\src-tauri\bundle-occt'
$definitions = @{
  'opencascade-7.7.0' = @{ name='Open CASCADE Technology'; version='7.7.0'; license='LGPL-2.1-only WITH OCCT-exception-1.0'; notices=@('licenses/OCCT-LICENSE_LGPL_21.txt', 'licenses/OCCT_LGPL_EXCEPTION.txt'); header='opencascade-7.7.0\inc\Standard.hxx' }
  'tbb_2021.5-vc14-64' = @{ name='oneTBB'; version='2021.5.0'; license='Apache-2.0'; notices=@('licenses/tbb-LICENSE-Apache-2.0.txt'); header='tbb_2021.5-vc14-64\include\oneapi\tbb\version.h' }
  'openvr-1.14.15-64' = @{ name='OpenVR'; version='1.14.15'; license='BSD-3-Clause'; notices=@('licenses/openvr-1.14.15-64-LICENSE') }
  'freeimage-3.17.0-vc14-64' = @{ name='FreeImage'; version='3.17.0'; license='FreeImage'; notices=@('licenses/freeimage-license-fi.txt'); header='freeimage-3.17.0-vc14-64\include\FreeImage.h' }
  'freetype-2.5.5-vc14-64' = @{ name='FreeType'; version='2.5.5'; license='FTL'; notices=@('licenses/freetype-LICENSE.TXT', 'licenses/freetype-FTL.TXT') }
  'ffmpeg-3.3.4-64' = @{ name='FFmpeg'; version='3.3.4'; license='LGPL-2.1-or-later'; notices=@('licenses/ffmpeg-LICENSE.md', 'licenses/ffmpeg-COPYING.LGPLv2.1.txt'); header='ffmpeg-3.3.4-64\include\libavutil\avutil.h' }
}
$groups = @{}
foreach ($line in Get-Content (Join-Path $occt 'THIRD-PARTY-NOTICES.txt')) {
  if ($line -match '^\s+(\S+\.dll)\s+\(([^)]+)\)\s*$') {
    $dll = $Matches[1]; $product = $Matches[2]
    if (-not $definitions.ContainsKey($product)) { throw "Unmapped native product: $product" }
    if (-not $groups.ContainsKey($product)) { $groups[$product] = @() }
    $groups[$product] += $dll
  }
}
$records = @()
# The same SDK that stage-occt resolved supplies these attribution headers.
$products = Split-Path (Split-Path $env:OCCT_INCLUDE_DIR -Parent) -Parent
foreach ($product in ($groups.Keys | Sort-Object)) {
  $definition = $definitions[$product]
  $notices = @($definition.notices)
  if ($definition.header) {
    $header = [IO.File]::ReadAllText((Join-Path $products $definition.header), [Text.Encoding]::GetEncoding(1252))
    $prefix = [regex]::Match($header, '(?ms)\A.*?(?=^#ifndef)').Value
    if (-not $prefix.Trim()) { throw "Missing SDK attribution header: $product" }
    $notice = "licenses/$product-COPYRIGHT.txt"
    [IO.File]::WriteAllText((Join-Path $occt $notice), $prefix, [Text.UTF8Encoding]::new($false))
    $notices += $notice
  }
  $records += [ordered]@{ name=$definition.name; version=$definition.version; license=$definition.license; files=@($groups[$product] | Sort-Object); notices=$notices }
}
Copy-Item (Join-Path $PSScriptRoot 'notice-sources\msvc-license.txt') (Join-Path $occt 'licenses\MSVC-LICENSE.txt') -Force
$crt = @(Get-ChildItem (Join-Path $occt 'crt') -Filter '*.dll' -File | Sort-Object Name)
if ($crt.Count -eq 0) { throw 'No staged C++ runtime' }
$records += [ordered]@{ name='Microsoft Visual C++ Runtime'; version=$crt[0].VersionInfo.FileVersion; license='LicenseRef-Microsoft-Visual-Cpp-Runtime'; files=@($crt | ForEach-Object { 'crt/' + $_.Name }); notices=@('licenses/MSVC-LICENSE.txt') }
$sysroot = & rustc --print sysroot
if ($LASTEXITCODE) { throw 'Cannot locate the Rust standard-library notices' }
$rustVersion = & rustc --version
if ($LASTEXITCODE) { throw 'Cannot identify the Rust standard library' }
$rustHtml = [IO.File]::ReadAllText((Join-Path $sysroot 'share\doc\rust\COPYRIGHT-library.html'))
$rustText = [Net.WebUtility]::HtmlDecode([regex]::Replace($rustHtml, '<[^>]+>', ''))
if ($rustText.Length -lt 10000) { throw 'Incomplete Rust standard-library copyright inventory' }
[IO.File]::WriteAllText((Join-Path $occt 'licenses\Rust-Std-COPYRIGHT.txt'), $rustText, [Text.UTF8Encoding]::new($false))
$records += [ordered]@{ name='Rust standard library (linked into simrep.exe and simrep-app.exe)'; version=$rustVersion; license='(MIT OR Apache-2.0) AND LicenseRef-Rust-Std-Bundled'; files=@('licenses/Rust-Std-COPYRIGHT.txt'); notices=@('licenses/Rust-Std-COPYRIGHT.txt') }
ConvertTo-Json -InputObject $records -Depth 8 | Set-Content (Join-Path $occt 'notice-manifest.json') -Encoding UTF8
& (Join-Path $root 'app\src-tauri\bundle-runtime\node.exe') (Join-Path $PSScriptRoot 'installer-notices.mjs')
if ($LASTEXITCODE) { throw "Installer notice generation failed: $LASTEXITCODE" }
& (Join-Path $root 'app\src-tauri\bundle-runtime\node.exe') (Join-Path $PSScriptRoot 'installer-notices.mjs') --check
if ($LASTEXITCODE) { throw "Installer notice verification failed: $LASTEXITCODE" }
