# SimRep AI bridge / AI 연결

SimRep can run a local AI agent with your own ChatGPT or Claude account. The
desktop host owns the Node process and opens OAuth in the system browser.
The WebView receives account labels and chat/tool events, never tokens.
The stable transport contract is [PROTOCOL.md](PROTOCOL.md).

SimRep는 사용자의 ChatGPT 또는 Claude 계정으로 로컬 AI 에이전트를 실행합니다.
로그인은 시스템 브라우저에서 진행되며 토큰은 WebView로 전달하지 않습니다.
AI 도구는 선택한 프로젝트 폴더의 SimRep MCP 서버만 사용합니다.
파일을 생성하거나 덮어쓰는 작업은 승인 창에서 허용해야 실행됩니다.

## Install / 설치

Use Node.js **18.18+**, install this directory's pinned dependencies, and build
the SimRep CLI. The SDK packages include platform-native binaries; install on
the target OS/architecture, not in WSL for a Windows release.

```sh
cd ai
npm ci
npm test
npm run typecheck
npm run licenses
cd ..
cargo build --release -p simrep-cli
export SIMREP_BIN=/absolute/path/to/target/release/simrep
```

For development, run the desktop app using its normal `app/` workflow. The Rust
host finds `ai/server.mjs` in the checkout; packaged builds bundle `ai/*.mjs`,
installed `ai/node_modules`, and `mcp/server.mjs` as resources. Install `ai/`
dependencies before building the desktop bundle. Node must be available on
`PATH`. The CLI must be installed separately and configured with `SIMREP_BIN`
for a packaged app. No login credential is embedded in a bundle.

개발 환경에서는 `ai/`에서 `npm ci`를 먼저 실행한 후 기존 `app/` 개발 명령을
사용합니다. 배포 앱에도 Node와 SimRep CLI가 필요합니다. `SIMREP_BIN`은 실제
CLI 실행 파일의 절대 경로로 설정합니다. 운영체제에 맞는 SDK 바이너리를 설치한
뒤 앱을 빌드하세요.

## ChatGPT

Choose **Continue with ChatGPT** to add an account. The first sign-in registers
SimRep using OpenAI's open-source dynamic-client flow. Returning sign-ins reuse
that account's issued client ID and this installation's stable host ID. Accounts
and workspace registrations stay separate, even when their email addresses match.
Choose an authenticated account when starting a chat.

Eligible ChatGPT **Plus and Pro** accounts can use their plan for eligible
Responses API requests. OpenAI currently describes plan usage as available to
open-source partners and selected private clients; commercial sign-in is a limited
trial. Login alone is not proof of eligibility or model access. Missing
`chatgpt.tokens.use.direct` permission blocks inference. A denied permission can
be requested again using the saved account. No automatic billing fallback occurs.
App/plan usage limits, region, workspace and policy restrictions still apply.

Show **Using ChatGPT plan** near the composer. On first successful plan sign-in,
explain that eligible requests use the user's plan, and link **Manage usage** to
[ChatGPT usage settings](https://chatgpt.com/settings/usage). Model IDs are passed
through; the conservative default is `gpt-5.4`, not an entitlement claim.

**Continue with ChatGPT**으로 계정을 연결합니다. 사용 가능한 Plus/Pro 계정의
구독 한도 또는 허용된 크레딧으로 적격 요청을 실행합니다. OpenAI 정책상 구독
사용은 오픈소스 파트너 및 선정된 비공개 클라이언트 대상이며, 로그인 성공이
모델 사용 권한을 보장하지 않습니다. 구독 사용 동의가 없으면 추론을 실행하지
않습니다. 한도는 ChatGPT 설정의 Usage에서 관리하세요.

Signing out stops that account's chats, attempts remote refresh-session revocation,
deletes its local credential record, and retains only its account/client mapping.
If remote revocation could not be confirmed, disconnect SimRep in ChatGPT settings.
The bridge returns `remoteRevocationConfirmed: false` so the UI can say so.
Temporary refresh failures preserve credentials; terminal refresh errors clear
unusable tokens and require login using the retained registration.

## Claude

Install the official [Claude Code CLI](https://code.claude.com/docs/en/setup)
and choose **Sign in with Claude**. SimRep launches `claude auth login`, which
owns the browser flow and credential storage. You can also run that command in a
terminal before starting SimRep. `SIMREP_CLAUDE_BIN` can name the official CLI
executable if it is not on `PATH`. Status comes from the official Agent SDK's
`query().accountInfo()` initialization handshake, without an inference request.
The chat provider uses `@anthropic-ai/claude-agent-sdk` `query()`, model `sonnet`
unless explicitly selected, and only the SimRep MCP server.

Alternatively, start the app with `ANTHROPIC_API_KEY` set. It stays in the process
environment and is never entered in the WebView or copied into SimRep's store.
API-key usage is billed through the linked Claude Console account, not treated as
Claude subscription OAuth. Remove the environment variable to sign out of this
mode. Claude CLI logout is shared with other Claude Code applications.

Anthropic's support article, updated October 7, 2026, says signed-in Claude plan
SDK and `claude -p` usage still draws from subscription limits; Max and Team can
also use linked Console monthly API credits through API keys. Availability and
plan limits are controlled by Anthropic. Its generic SDK overview additionally
states that third-party products offering claude.ai login/rate limits require
prior approval. This integration uses the official CLI/SDK rather than a custom
OAuth client; distributing it does **not** itself establish Anthropic approval.
Confirm applicable terms/approval for your distribution and use the API-key path
where required. SimRep never reads Claude Code's OAuth credential files.

공식 Claude Code CLI를 설치하고 **Sign in with Claude**를 누르면
`claude auth login`이 실행됩니다. 터미널에서 미리 로그인해도 됩니다.
Claude 토큰은 공식 CLI/SDK만 관리하며 SimRep가 추출하거나 복사하지 않습니다.
`ANTHROPIC_API_KEY` 환경변수도 지원하며 API 결제 경로임을 구분합니다.
Anthropic 지원 문서에는 구독 로그인 상태의 SDK 사용이 구독 한도에 반영된다고
명시되어 있지만, 일반 SDK 문서는 제3자 제품의 claude.ai 로그인 제공에 사전
승인이 필요하다고 설명합니다. 배포 정책과 승인 여부를 확인하고 필요한 경우
API 키 경로를 사용하세요.

## Security and recovery / 보안

- Every ChatGPT attempt has fresh state, nonce and S256 PKCE. Its listener binds
  only `127.0.0.1` on an ephemeral port, at `/auth/callback`, and expires in ten
  minutes. Callback state, exact URI, issued client ID and one-time use are checked.
- ID tokens use `jose`, OpenAI JWKS and RS256 verification of issuer, audience,
  expiration, nonce and selected account identity. Granted direct-use scope is
  checked before inference and after refresh.
- The store defaults to the OS app-config directory `dev.simrep.app/ai` (Tauri
  supplies its platform app-config directory). `SIMREP_AI_CONFIG_DIR` is an
  administrator/runtime setting, not a WebView parameter. Repository paths and
  symlinked config directories are refused. Files are atomically replaced with
  `0600` permissions and directories `0700` on Unix. Windows uses the user's app
  data directory and its inherited per-user ACL; chmod is not a Windows ACL API.
- `account-<hash>.json` contains credentials; `registration-<hash>.json` contains
  only account/client identity; `host.json` stores the installation ID. Credential
  files and provider stderr never enter logs or protocol error messages.
- Store mutations/refreshes are serialized in-process and protected by an
  exclusive cross-process `session.lock`. After a crash, verify that **all** AI
  sidecars are stopped before manually removing a stale lock. It is never stolen
  automatically, avoiding a rotating-token race.
- Codex receives `ACCESS_TOKEN` only through its child environment, uses a
  Responses provider at `https://api.openai.com/v1`, and is restarted with
  `thread/resume` after token renewal. Its private conservative model catalog
  removes shell and file-edit capabilities; built-in browser, exec, agents,
  plugins, web search and hooks are disabled. Claude uses `tools: []`, strict MCP
  configuration and no project/user settings. Codex's working directory remains
  in its private app config runtime, so project-local `.codex` settings cannot
  launch additional MCP servers; only the SimRep proxy sees the project path.
- Both agents receive an MCP approval proxy. Only fixed SimRep tool names are
  accepted; all non-read-only tools require a UI decision before forwarding.
  Denial, timeout and cancellation fail closed. Provider built-in permissions
  cannot bypass the proxy. The actual MCP server resolves paths and symlinks
  inside `SIMREP_WORKSPACE`; no shell execution is offered. Project directories
  containing known AI credential stores cannot be used as workspaces.
- Treat assistant/tool text as untrusted text, not executable markup. AI sends
  prompts and tool results to the selected provider under its data policy.

로그인 정보는 OS 앱 설정 폴더에만 보관합니다. 로그아웃 시 로컬 토큰을 삭제합니다.
네트워크 문제로 원격 해제가 확인되지 않으면 ChatGPT 설정에서 앱 연결도 해제하세요.
권한 승인 없이 파일을 변경하는 MCP 호출은 실행하지 않습니다.

## Verification / 검증

```sh
cd ai
npm test
npm run typecheck
# Explicit real-login smoke; reads only a temporary CSV and may consume quota:
SIMREP_AI_REAL_SMOKE=claude npm run smoke:real
# or SIMREP_AI_REAL_SMOKE=chatgpt npm run smoke:real
cd ..
CARGO_TARGET_DIR=/tmp/simrep-ai-tauri cargo test --manifest-path app/src-tauri/Cargo.toml
CARGO_TARGET_DIR=/tmp/simrep-ai-tauri cargo clippy --manifest-path app/src-tauri/Cargo.toml --all-targets -- -D warnings
SIMREP_BIN=/absolute/path/to/simrep node mcp/smoke.mjs
```

Ordinary tests use a local fake JWKS and a fake Agent SDK driving the real MCP
server, so they need no credentials and consume no quota. The real test reports
an explicit skip without the opt-in variable or an eligible authenticated account.
Interactive browser consent, real account eligibility, provider billing/limits,
Windows ACLs/browser launch and packaged SDK binaries require platform/login
testing. A successful initialized provider or model catalog is not an inference
entitlement check.

## Dependency licences / 의존성 라이선스

Direct dependencies are pinned exactly and transitive versions are recorded in
`package-lock.json`:

| Package | Version | Licence / terms |
| --- | --- | --- |
| `@anthropic-ai/claude-agent-sdk` | 0.3.311 | `SEE LICENSE IN README.md`; Anthropic Commercial Terms, **not** an OSI open-source licence |
| `@openai/codex` | 0.164.0 | Apache-2.0 |
| `jose` | 5.10.0 | MIT |
| `typescript` (development) | 5.9.3 | Apache-2.0 |
| `@types/node` (development) | 18.19.130 | MIT |

Keep package licence notices in redistributed `node_modules`. The Agent SDK's
bundled native components remain subject to its terms; do not describe the entire
AI dependency bundle as permissively licensed. The existing SimRep MCP adapted
file remains subject to the repository's `licenses/THIRD_PARTY.md`.
The installed inventory check (`npm run licenses`) reports permissive MIT,
Apache-2.0, BSD, ISC and Unlicense packages plus the two Anthropic SDK/native
packages requiring the commercial-terms review above.

## Authoritative references

- [OpenAI quickstart](https://developers.openai.com/siwc/quickstart)
- [OSS registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)
- [Accounts and refresh](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions)
- [Errors and recovery](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery)
- [Codex app-server](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server)
- [OpenAI UI guidelines](https://developers.openai.com/siwc/ui-ux-guidelines)
- [Claude Agent SDK TypeScript reference](https://code.claude.com/docs/en/agent-sdk/typescript)
- [Claude plan SDK usage](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
- [Agent SDK repository and terms](https://github.com/anthropics/claude-agent-sdk-typescript)
