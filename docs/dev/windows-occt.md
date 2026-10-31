# Windows OCCT SDK for the simrep-cad `occt` feature

Verified October 23, 2026 on this workstation: Windows `cargo.exe` 1.97.1
(MSVC 2022, `x86_64-pc-windows-msvc`), Ryzen 9 5950X. `cargo.exe test -p
simrep-cad --features occt` passes 5/5 tests and `simrep.exe refine-geometry`
solves a generated STEP sphere with the exact B-rep path on Windows.

## SDK choice

OCCT **7.7.0** official prebuilt Windows x64 archive installer, from the
Open-Cascade-SAS GitHub release
<https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V7_7_0>:

- Asset: `opencascade-7.7.0-vc14-64.exe`, 270,504,984 bytes.
- SHA-256 (computed here; upstream publishes no checksum for this asset):
  `d172eb4a2d5f291291b82327bb522037c5fc7d204aca815bb3062d6bd880abd5`.
- `vc14-64` binaries are MSVC 2017-toolset builds; they link and run fine
  against MSVC 2022 (`vc143`) projects because `vc14x` toolsets are
  ABI-compatible within one C++ runtime.
- V7_6_x releases have no downloadable Windows binaries at all; V7_7_0 is the
  newest 7.6/7.7-series release with one. The binding targets OCCT 7.6/7.7
  toolkit names, so 7.8+ archives (`occt-vc143-64.zip`) are not usable.

Install root: `%USERPROFILE%\occt\` (user-local; no admin rights, no system
PATH or registry changes). CASROOT is
`%USERPROFILE%\occt\opencascade-7.7.0`; bundled third-party directories
(freetype, freeimage, tbb, openvr, ffmpeg, draco, rapidjson) sit next to it
because `custom.bat` resolves them as `%~dp0..\<name>`.

## Install: extract the installer, do not run it

The `.exe` is an Inno Setup 6.0.0 installer. On this machine its
`/VERYSILENT /CURRENTUSER` headless run stalled (the launcher and its
`is-*.tmp` child stayed idle for minutes and wrote nothing), so do **not** run
it unattended. Extract the payload with innoextract 1.9
(<https://github.com/dscharrer/innoextract/releases>) instead; extraction
writes files only - no registry entries, no uninstaller, no PATH changes.
Both steps ran fine without admin rights.

From WSL, with the installer already downloaded into a new empty
`%USERPROFILE%\occt-download\`:

```bash
cd /mnt/c/Users/<user>/occt-download
curl -fSL -o innoextract-1.9-windows.zip \
  https://github.com/dscharrer/innoextract/releases/download/1.9/innoextract-1.9-windows.zip
cd /mnt/c && timeout 60 powershell.exe -NoProfile -NonInteractive -Command \
  'Expand-Archive -Force %USERPROFILE%\occt-download\innoextract-1.9-windows.zip %USERPROFILE%\occt-download\innoextract-1.9-windows' </dev/null
```

Then extract the SDK subset into `%USERPROFILE%\occt` (payload files live
under an `app\` prefix inside the installer; the `-I` patterns must use
**backslashes** on Windows - forward slashes silently match nothing):

```bash
cd /mnt/c && timeout 900 /mnt/c/Users/<user>/occt-download/innoextract-1.9-windows/innoextract.exe \
  -d '%USERPROFILE%\occt' \
  -I 'app\opencascade-7.7.0\env.bat' -I 'app\opencascade-7.7.0\custom.bat' \
  -I 'app\opencascade-7.7.0\draw.bat' -I 'app\opencascade-7.7.0\genconf.bat' \
  -I 'app\opencascade-7.7.0\genproj.bat' -I 'app\opencascade-7.7.0\msvc.bat' \
  -I 'app\opencascade-7.7.0\LICENSE_LGPL_21.txt' \
  -I 'app\opencascade-7.7.0\OCCT_LGPL_EXCEPTION.txt' \
  -I 'app\opencascade-7.7.0\README.txt' \
  -I 'app\opencascade-7.7.0\inc' -I 'app\opencascade-7.7.0\win64' \
  -I 'app\opencascade-7.7.0\data' -I 'app\opencascade-7.7.0\src' \
  -I 'app\freetype-2.5.5-vc14-64' -I 'app\freeimage-3.17.0-vc14-64' \
  -I 'app\tbb_2021.5-vc14-64' -I 'app\openvr-1.14.15-64' \
  -I 'app\ffmpeg-3.3.4-64' -I 'app\rapidjson-1.1.0' -I 'app\draco-1.4.1-vc14-64' \
  '%USERPROFILE%\occt-download\opencascade-7.7.0-vc14-64.exe' </dev/null
# Move app\* up so the layout matches a normal install:
cd /mnt/c && timeout 300 powershell.exe -NoProfile -NonInteractive -Command \
  'Get-ChildItem %USERPROFILE%\occt\app -Directory | Move-Item -Destination %USERPROFILE%\occt; Remove-Item %USERPROFILE%\occt\app' </dev/null
```

This skips `doc/`, `tests/`, `samples/`, `adm/`, `dox/`, `tools/` and the Qt,
VTK and tcltk third-party trees (~60k files, several GB) which the CAD
exchange path does not need. `custom.bat` still lists the skipped trees in
its `CSF_OPT_*` PATH sets; env.bat then appends a few nonexistent directories
to PATH, which is harmless. `src/` is kept: it carries OCCT runtime
resources (shaders, STEP/IGES resource files) that some toolkits locate
relative to CASROOT. Result: 77 DLLs and 77 import libraries under
`opencascade-7.7.0\win64\vc14\{bin,lib}`, including every `TK*` name in
`crates/simrep-cad/build.rs`.

Measured times on this machine: download ~4 min (270 MB), innoextract run
<1 min, folder move <1 s.

## Build

Same recipe as `docs/dev/windows-gpu.md`: foreground from `/mnt/c`, closed
stdin, `timeout`, output piped. `CARGO_TARGET_DIR` on C:, `WSLENV` forwarding.
The build script reads `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR` as Windows
paths; forwarding them with WSLENV passes them verbatim (no `/p` flag).

```bash
export CARGO_TARGET_DIR='%USERPROFILE%\simrep-target-cad'
export OCCT_INCLUDE_DIR='%USERPROFILE%\occt\opencascade-7.7.0\inc'
export OCCT_LIB_DIR='%USERPROFILE%\occt\opencascade-7.7.0\win64\vc14\lib'
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR:OCCT_INCLUDE_DIR:OCCT_LIB_DIR"

cd /path/to/simrep/repo
MANIFEST="$(wslpath -w "$PWD/Cargo.toml")"
cd /mnt/c
timeout 1800 /mnt/c/Users/<user>/.cargo/bin/cargo.exe build \
  --manifest-path "$MANIFEST" \
  -p simrep-cad --features occt </dev/null 2>&1 | tr -d '\r'

timeout 1800 /mnt/c/Users/<user>/.cargo/bin/cargo.exe build \
  --manifest-path "$MANIFEST" \
  -p simrep-cli --features cad </dev/null 2>&1 | tr -d '\r'
```

Measured here: `simrep-cad --features occt` first build 49.14 s;
`simrep-cli --features cad` first build (pulls wgpu through simrep-fly)
about 6 min; incremental rebuild 0.46 s. No crates/ changes were needed -
the existing `OCCT_INCLUDE_DIR` / `OCCT_LIB_DIR` variables work as-is, and
the debug binary links the release OCCT import libraries without mismatch
(vc14-64 ships release DLLs only).

## Runtime: tests and the CLI need OCCT DLLs on PATH

`env.bat` gives the complete runtime closure (OCCT `win64\vc14\bin` plus the
bundled third-party `bin` dirs for freetype, freeimage, tbb, openvr, ffmpeg,
draco). Call it inside the same `cmd.exe` as the command so the PATH change
stays scoped to that process:

```bash
cd /mnt/c
timeout 1800 cmd.exe /c "call %USERPROFILE%\occt\opencascade-7.7.0\env.bat vc143 win64 release >nul & %USERPROFILE%\.cargo\bin\cargo.exe test --manifest-path $MANIFEST -p simrep-cad --features occt" \
  </dev/null 2>&1 | tr -d '\r'
```

`vc143` and `vc14` are equivalent here - env.bat maps both to the
`win64\vc14` folders. Measured: first `cargo.exe test` run (compiles the
test binaries) about 2 min, then `test result: ok. 5 passed; 0 failed` for
`tests\import.rs`; a warm rerun finishes in ~1 s.

A `refine-geometry` run on a STEP fixture generated by `simrep-cad`'s
`write_fixture` (shape 0 = radius-0.31 sphere, written on WSL where the
generator is easiest to build, copied to
`%USERPROFILE%\simrep-cad-fixtures\`):

```bash
cd /mnt/c
timeout 300 cmd.exe /c \
  'call %USERPROFILE%\occt\opencascade-7.7.0\env.bat vc143 win64 release >nul & %USERPROFILE%\simrep-target-cad\debug\simrep.exe refine-geometry %USERPROFILE%\simrep-cad-fixtures\sphere.step --grid 33,33,33 --spacing 0.05,0.05,0.05 --origin -0.8,-0.8,-0.8 --electrode sphere=1:120 --cad-mode exact --out %USERPROFILE%\simrep-cad-fixtures\sphere' \
  </dev/null 2>&1 | tr -d '\r'
```

Measured output: `CAD part 0 "sphere": Exact`, converged in 21 cycles
(relative_residual 3.5e-11), `compute_ms=2069.6`,
`preparation_ms=28340.6`, `end_to_end_ms=30420.7`; produced `.pa0`, `.pa1`,
`.pa#` and `.cuts.json`. Exact-mode OCCT preparation is dominated by B-rep
extrema/classification queries, same as on Linux.

## Notes and leftover state

- The default Windows build (`cargo.exe build` without `--features occt` /
  `cad`) does not need any of this and still works.
- Extracted-only install means no `unins000.exe`, no Start-menu entries, no
  registry writes. Deleting `%USERPROFILE%\occt` removes the SDK. The
  download folder `%USERPROFILE%\occt-download` (installer + innoextract)
  can also be deleted once done.
- The stalled silent installer run was killed with `taskkill /T /F`; it had
  created only a `%USERPROFILE%\AppData\Local\Temp\is-*.tmp` staging dir.
- WSL `/usr/lib/x86_64-linux-gnu/libTK*.so` files cannot be linked into an
  MSVC executable; this SDK is the Windows-side counterpart of the Ubuntu
  7.6.3 dev packages used on WSL.
