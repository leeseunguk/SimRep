# Self-contained Windows NSIS build

Build from a Windows-side source export, using Windows Node/npm and MSVC Cargo.
The Windows bundle uses three configs: `tauri.conf.json`,
`tauri.cad.conf.json`, and `tauri.windows.conf.json`. Do not copy Linux
`node_modules` into it.

1. In `app/`, run `npm ci` and `npm run build`.
2. In `ai/`, run `npm ci`, `npm test`, `npm run typecheck`, and
   `npm run licenses`; then `npm prune --omit=dev`.
3. Set `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR` as described in `windows-occt.md`.
   Build `cargo build --release -p simrep-cli --features cad` at the root.
4. Run `tools/win/stage-occt.ps1 -Dest app/src-tauri/bundle-occt` for OCCT
   and its C++ runtime closure **before** staging the runtime. Then run
   `tools/win/stage-runtime.ps1 -Dest app/src-tauri/bundle-runtime
   -Cli <target>/release/simrep.exe`. This downloads Node 24.14.1 Windows x64,
   verifies the pinned archive SHA-256, and stages only `node.exe`, its complete
   upstream licence, and the CLI. `-Archive` accepts a previously downloaded
   archive with the same required checksum. Runtime staging calls
   `stage-notices.ps1` and fails if complete notice generation or verification fails.
5. From `app/`, run `npm run tauri -- build --features cad --bundles nsis
   --config src-tauri/tauri.cad.conf.json
   --config src-tauri/tauri.windows.conf.json`. Its prebuild command checks that
   the notices still match the staged payload. Do not override this check.

Release Windows builds launch the bridge with `runtime/node.exe`, set its
`SIMREP_BIN` to the installed sibling `simrep.exe`, and prepend the private
runtime directory to the bridge's PATH so nested MCP/provider processes also
use it. Debug builds retain the developer's system Node. No global Node,
Python, npm, or CLI installation is needed for the AI account-status smoke.
Some optional analysis tools still require Python as before.

Tauri's canonical Windows resource paths have a verbatim `\\?\` prefix.
The native bridge uses `dunce::simplified` for the script and resource root
before passing them to Node, the CLI environment and PATH. Node 24.14.1 otherwise
exits with `EISDIR: lstat 'C:'` before processing any RPC. The Windows-only
`ai::tests::node_entrypoint_does_not_use_a_windows_verbatim_prefix` regression
covers this conversion, including a user directory containing a space.

The Windows overlay places the 39 OCCT/runtime-dependency DLLs next to both
executables, alongside the three C++ runtime DLLs. Unlike the app, the CLI has
ordinary startup imports: keeping all of OCCT only under `occt/` would prevent
it from starting. The app accepts the shared executable directory as well as
its existing `occt/` and SDK search paths. Licence files remain under `occt/`.
There is no duplicate DLL tree and no global PATH modification.

A portable Node resource is deliberately used instead of SEA or JS bundling:
the existing SDKs rely on dynamic imports and package-relative native binaries.
The npm packages remain intact, but some upstream packages omit licence files.
`tools/win/installer-notices.mjs` inventories every installed production npm
package instance (including nested/native packages), the frontend production
dependency closure, and both locked Windows Cargo closures with CAD enabled.
Build and proc-macro crates are included conservatively. It copies every
LICENSE/LICENCE/COPYING/NOTICE/COPYRIGHT text and text in licence directories,
supplements known omissions with version-mapped upstream snapshots, and
otherwise uses checked-in SPDX standard text with the package's metadata author
attribution, explicitly marking the missing package text. Missing metadata/text,
empty notices, unmapped DLLs, and changed source snapshot checksums fail the build.
Node's entire upstream LICENSE includes its embedded third parties. Every staged
OCCT/dependency/CRT DLL must have a full-text mapping. The linked Rust standard
library's complete copyright inventory is included too. Commercial provider
notices retain their own terms and are not relabelled MIT.

Codex's nested voice libraries, Microsoft runtime, ripgrep and its PCRE2 are
mapped separately. The frozen SDK-embedded snapshot also covers FreeType's BDF,
PCF and zlib code and FreeImage's JPEG, JPEG XR, OpenJPEG, PNG, LibRaw, TIFF,
WebP, OpenEXR, IlmBase and zlib code. It preserves full licence files, README
licences and unique source copyright/licence comments from FreeImage 3.17.0's
unmodified source archive. `python3 tools/win/refresh-sdk-embedded-notices.py
FreeImage3170.zip` verifies its pinned upstream SHA-256 before regenerating the
snapshot; the FreeType sources are pinned to `VER-2-5-5`. Ordinary builds use
these checked-in snapshots offline. This notice audit does not authenticate the
local OCCT SDK binaries; retain the SDK's provenance with release evidence.

The generated `licenses/THIRD_PARTY_NOTICES_INSTALLER.txt` and matching JSON
inventory are installed with `licenses/THIRD_PARTY.md`; they are build outputs,
not committed inputs. The JSON records package name/version/SPDX expression,
payload location, notice source, size and SHA-256. `node ai/license-check.mjs`
uses the same full-text npm check; `node tools/win/installer-notices.mjs --check`
compares the entire output with the current payload. Maintainers can refresh
reviewed supplemental/SPDX snapshots with `node tools/win/refresh-notice-sources.mjs`
(on Linux with Python 3 for Microsoft's DOCX). Review the package-to-notice map,
source manifest and texts before accepting a refresh; builds do not fetch
unpinned licence content. Node's complete licence is also installed as
`runtime/LICENSE`. Node plus its licence add 91,586,176 uncompressed bytes.
Measure compressed and installed sizes in the accompanying QA report rather
than reusing a historical CAD-only size.

The per-user NSIS installer supports `/S /D=C:\path` (the `/D` argument must be
last). Close the app and its owned WebView/AI process tree before
`uninstall.exe /S`. Wait for the copied uninstaller's full process tree, not
only the bootstrap process. Application data is retained by the existing
uninstall policy. Installer size, Node/CLI raw size, source identity, installed
scenarios and cleanup evidence belong in the build's QA report. The installed
MCP smoke must run `runtime/node.exe mcp/server.mjs` from the install folder,
initialize, and list all 23 tools including `optimize_design`.

When starting Windows commands from WSL, use a `/mnt/c` working directory,
closed stdin and an explicit timeout; pass cross-boundary environment variables
through `WSLENV`. Keep app/build processes inside a bounded foreground call and
terminate only stuck task-owned trees with `taskkill /T /F`.
`tools/win/winproc.ps1 cleanup` reports and removes orphan conhosts, never live user jobs.
