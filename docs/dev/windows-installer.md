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
4. Run `tools/win/stage-runtime.ps1 -Dest app/src-tauri/bundle-runtime
   -Cli <target>/release/simrep.exe`. This downloads Node 24.14.1 Windows x64,
   verifies the pinned archive SHA-256, and stages only `node.exe`, its complete
   upstream licence, and the CLI. `-Archive` accepts a previously downloaded
   archive with the same required checksum. Run `tools/win/stage-occt.ps1
   -Dest app/src-tauri/bundle-occt` for OCCT and its C++ runtime closure.
5. From `app/`, run `npm run tauri -- build --features cad --bundles nsis
   --config src-tauri/tauri.cad.conf.json
   --config src-tauri/tauri.windows.conf.json`.

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
The npm packages and their licence files remain intact. Node's complete licence
is installed as `runtime/LICENSE`; project/provider notices are installed as
`licenses/THIRD_PARTY.md`. Runtime files are staged, not checked into git.
Node plus its licence add 91,586,176 uncompressed bytes. The compressed package
and complete installed sizes are measured in the accompanying QA report.

The per-user NSIS installer supports `/S /D=C:\path` (the `/D` argument must be
last). Close the app and its owned WebView/AI process tree before
`uninstall.exe /S`. Wait for the copied uninstaller's full process tree, not
only the bootstrap process. Application data is retained by the existing
uninstall policy. Installer size, Node/CLI raw size, source identity, installed
scenarios and cleanup evidence belong in the build's QA report.

When starting Windows commands from WSL, use a `/mnt/c` working directory,
closed stdin and an explicit timeout; pass cross-boundary environment variables
through `WSLENV`. `tools/win/winproc.ps1 cleanup` reports and removes orphan
conhosts, never live user jobs.
