# SimRep AI bridge / AI 연결

The desktop app runs a local Node bridge using your selected provider account.
Rust opens the system browser; only account display fields, public device codes
and chat/tool events reach the WebView. All tools go through the same
workspace-restricted SimRep MCP approval proxy. The contract is
[PROTOCOL.md](PROTOCOL.md).

SimRep은 선택한 제공자 계정으로 로컬 AI 브리지를 실행합니다. 시스템 브라우저는
Rust 호스트만 열며, 토큰과 API 키는 WebView로 전달하지 않습니다. 기기 로그인
코드는 화면에 표시합니다. AI 도구는 선택한 프로젝트의 MCP 서버만 사용하고,
파일을 바꾸는 도구는 기존 승인 창에서 허용해야 실행됩니다.

## Install / 설치

Node.js **24+** is required by the pinned MIT native-provider adapter.
Install on the target OS/architecture: SDK packages contain native executables.

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

For development use the normal `app/` workflow. Bundles include `ai/*.mjs`,
`ai/auth/`, `ai/providers/`, installed `ai/node_modules`, and `mcp/server.mjs`.
Node and the CLI must be installed on the target machine; set `SIMREP_BIN` for a
packaged app. Never bundle login credentials.

개발 전 `ai/`에서 `npm ci`를 실행하세요. 배포 대상 운영체제에서 SDK를 설치한
후 앱을 빌드해야 합니다. 배포 앱에도 Node.js 24 이상과 SimRep CLI가 필요하며
`SIMREP_BIN`은 실제 CLI의 절대 경로로 지정합니다.

## Providers, billing and availability / 제공자와 결제

`auth.providers` is the authoritative table. Model lists are defaults, not a
promise of access. No provider switches silently to another billing path.
Environment keys explicitly select API billing and take precedence over OAuth.
Use an account and model you are entitled to use; the provider controls limits
and applicable terms. A reusable MIT implementation is not a provider approval
or an inference entitlement.

| ID | Sign-in | Inference wire | Tool path / account note |
| --- | --- | --- | --- |
| `chatgpt` | Browser PKCE | Codex Responses, through Codex app-server | Continue with ChatGPT; subscription client, account-scoped routing |
| `claude` | Anthropic browser PKCE, or `ANTHROPIC_API_KEY` | Claude Agent SDK | Only SimRep MCP; billing distinction below |
| `github-copilot` | GitHub device code | Chat Completions | GitHub grant mints short-lived Copilot tokens; only enabled tool-capable chat models |
| `xai` | xAI device code, or `XAI_API_KEY` | Responses | SuperGrok/X eligibility and limits are provider-controlled |
| `openrouter` | Browser PKCE, or `OPENROUTER_API_KEY` | Chat Completions | OAuth mints a permanent API key billed from OpenRouter credits, not subscription limits |
| `kimi-coding` | Regional device code, or `KIMI_API_KEY` | Anthropic-compatible Messages | Mainland `kimi.com` or global `kimi.ai`; saved region controls refresh and inference |
| `devin` | Browser PKCE, fixed loopback port 59653 | Cascade Connect (`devin-agent`) | Uses the pinned MIT wire adapter, not the login-only `api.devin.ai` REST host |
| `meta` | Meta device code, or `META_API_KEY` | Responses | Identity grant mints a Model API key; refresh re-mints that key |
| `kiro` | Social browser PKCE | CodeWhisperer `generateAssistantResponse` AWS eventstream | MIT community protocol; Kiro credits, profile ARN and server model access apply |
| `cursor` | Browser PKCE + token-release polling | Native HTTP/2 Connect | **Sign-in only**; SimRep has no verified MCP-only exec-channel adapter |
| `radius` | Gateway device code | Native `pi-messages` | **Sign-in only**; dynamic model/transport contract not verified with this approval proxy |

Cursor/Radius sign-in is useful for validating/storing their grants, but the UI
marks them sign-in-only and `chat.start` refuses them with `PROVIDER_UNSUPPORTED`.
No shell/file execution channel is exposed as a substitute.

Kiro uses the MIT `eddieparc/omo-accounts` community implementation as its source,
not omo's bundled plugin. The browser portal presents social account sign-in;
this bridge uses the social PKCE path, not AWS Builder ID device authorization.
It neither imports nor reads `~/.omo/agent/auth.json`. Kiro model access and
credits remain server-controlled; no real account entitlement is inferred from
the catalog.

Copilot currently targets github.com, not a user-configured enterprise auth
host. It does not enable new model policies silently: enable a tool-capable
model with GitHub first. The credential-specific API endpoint and model list
are refreshed with the short-lived Copilot token.

Kimi's region picker accepts `mainland-cn` and `global`. `KIMI_CODE_REGION`
sets the default. A saved global grant always refreshes at `auth.kimi.ai` and
uses `api.kimi.ai/coding`, even if that variable changes. Subscription calls
include the source client's stable device identity headers.

제공자 목록과 로그인 종류는 브리지의 `auth.providers`에서 가져옵니다. API 키는
환경변수로만 설정하며 입력 창은 없습니다. OpenRouter OAuth는 구독 한도가 아니라
크레딧으로 결제되는 키를 만듭니다. Kimi Code는 계정이 있는 지역을 선택하세요.
Cursor와 Radius는 로그인만 지원하고 대화 시작은 거부합니다. Kiro는 MIT 커뮤니티
구현을 바탕으로 소셜 PKCE와 MCP 도구를 연결하며, 기존 omo 토큰은 읽지 않습니다.

### Generic API-key table / API 키 제공자

Set the environment before starting the desktop app, then restart it. Keys are
never entered in a WebView, returned by protocol methods or copied to the store.
Aliases make API billing separate from a subscription/OAuth provider selection.

| ID | Environment | Wire |
| --- | --- | --- |
| `openai` | `OPENAI_API_KEY` | Responses |
| `gemini` | `GEMINI_API_KEY` | Gemini's OpenAI-compatible Chat Completions |
| `deepseek` | `DEEPSEEK_API_KEY` | Chat Completions |
| `mistral` | `MISTRAL_API_KEY` | Chat Completions |
| `groq` | `GROQ_API_KEY` | Chat Completions |
| `zai` | `ZAI_API_KEY` | GLM PAYG Chat Completions |
| `qwen` | `DASHSCOPE_API_KEY` | International DashScope-compatible Chat Completions |
| `minimax` | `MINIMAX_API_KEY` | Anthropic-compatible Messages |
| `openrouter-key` | `OPENROUTER_API_KEY` | Chat Completions |
| `xai-key` | `XAI_API_KEY` | Responses |
| `anthropic-key` | `ANTHROPIC_API_KEY` | Anthropic Messages (direct, without Agent SDK) |

Default endpoints/models live in `providers/table.mjs`. API clients offer only
MCP function tools; no provider-hosted shell, browser or file tools are declared.
They preserve the conversation and tool results for the next round, fail closed
on malformed/truncated tool calls, and cap a turn at 32 tool rounds.

## ChatGPT

Choose **Continue with ChatGPT**. The default is omo's maintained
`chatgpt-subscription` flow: the Codex public client, PKCE/state/nonce, a listener
bound to `127.0.0.1`, and a `localhost` callback on port 1455 or 1457.
Occupied listeners are not cancelled or taken over. If both are busy, close
the other sign-in and retry. Verified identity and distinct account/client
registrations remain separate even when email addresses match.

The Codex app-server receives the access token only through its child
environment and uses `https://chatgpt.com/backend-api/codex/responses` plus the
credential's ChatGPT account header. Subscription usage and model access are
controlled by OpenAI. Login is not a claim that a plan supports every model.

The original dynamic-client SIWC flow remains available explicitly through
`SIMREP_CHATGPT_AUTH_FLOW=siwc`. It registers an open-source dynamic client and
targets `https://api.openai.com/v1/responses`; inference requires the
`chatgpt.tokens.use.direct` grant. SIWC registrations are retained but are not
mixed with Codex-client registrations. Its local contract/JWKS/refresh tests
pass; no real SIWC eligibility is claimed.

Sign-out stops chats, attempts remote revocation, deletes local credentials,
and preserves non-secret registration. If remote revocation is unconfirmed,
disconnect the application in ChatGPT settings. **Manage usage** is a host-owned
action opening [ChatGPT usage settings](https://chatgpt.com/settings/usage).

**Continue with ChatGPT**은 기본적으로 Codex 구독 PKCE 경로를 사용합니다.
기존 SIWC 경로는 `SIMREP_CHATGPT_AUTH_FLOW=siwc`로 명시해서 선택할 수 있습니다.
서로 다른 클라이언트의 계정 등록은 섞지 않습니다. 요금제와 모델 권한은 OpenAI가
결정하며, 실계정 권한을 자동으로 검증하거나 다른 결제로 전환하지 않습니다.

## Claude: omo PKCE + Agent SDK / Claude 연결

Choose **Sign in with Claude**. SimRep runs the same Anthropic OAuth client,
scope and PKCE token exchange as omo's MIT `anthropic-subscription` auth source,
then passes the selected access token as `CLAUDE_CODE_OAUTH_TOKEN` to the official
Agent SDK. It prefers loopback port 53692 and falls back to an ephemeral port;
the authorization URL names the actual listening port. SimRep owns refresh
rotation and stores credentials in its app-config directory. It does not run
`claude auth login` or read the host CLI login. Claude SDK settings and transcript
files use a separate private `claude-sdk/` directory.

`ANTHROPIC_API_KEY` remains an alternative, passed only to the SDK child.
Environment API-key billing is separate from Claude OAuth. Remove that variable
and restart to leave key mode; sign-out cannot change the parent environment.

The installed omo/senpi `docs/providers.md`, **Claude Pro/Max**, says:

> Third-party harness usage draws from extra usage and is billed per token, not against Claude plan limits.

Its [extra usage link is https://claude.ai/settings/usage](https://claude.ai/settings/usage).
This is an attribution of **omo's wording**, not a guarantee of Anthropic billing.
Anthropic's [Use the Claude Agent SDK with your Claude plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)
article, inspected October 27, 2026, describes signed-in plan SDK usage as drawing
from subscription limits. The documents differ; the account's usage settings,
applicable Anthropic terms and first-party policy determine actual billing.
The [Agent SDK overview](https://platform.claude.com/docs/en/agent-sdk/overview)
also describes requirements for third-party products offering Claude account
login. Reusing a public client does not establish distribution approval.

**Sign in with Claude**을 누르면 SimRep이 omo와 같은 Anthropic PKCE 로그인을
직접 수행한 뒤 선택한 자격 증명을 Claude Agent SDK에 전달합니다.
`claude auth login`은 실행하지 않으며 호스트 CLI의 토큰을 읽지 않습니다.
`ANTHROPIC_API_KEY` 대안은 그대로 지원하고 API 결제 경로로 표시합니다.

omo 자체 문서의 위 문구는 **제3자 하네스 사용이 Claude 요금제 한도가 아니라
토큰당 결제되는 추가 사용량(extra usage)에서 차감된다**는 설명입니다.
사용량 화면은 [https://claude.ai/settings/usage](https://claude.ai/settings/usage)입니다.
이는 omo 문서의 설명을 인용한 것이며 결제를 보장한다는 뜻은 아닙니다.
2026년 10월 27일 확인한 [Anthropic 지원 문서](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)는
로그인한 플랜의 SDK 사용이 구독 한도에 반영된다고 설명하여 서로 차이가 있습니다.
실제 결제와 배포 조건은 계정의 사용량 화면 및 Anthropic의 현행 정책을 따릅니다.

## Security and recovery / 보안

- Every browser attempt has fresh S256 PKCE, state and nonce. Listeners bind
  only `127.0.0.1`; exact host/path, one-time code and state are checked.
  OpenRouter does not return state, so its unpredictable callback path is the
  CSRF binding and PKCE binds the exchanged code. Browser attempts expire after
  ten minutes. Devin requires port 59653; Anthropic accepts the actual fallback
  port. Kiro social callbacks use localhost and a loopback listener.
- Device polling obeys `authorization_pending`, `slow_down`, expiry, denial and
  cancellation. Private device codes never reach UI events. Verification URLs
  must be fixed first-party HTTPS origins; Rust validates browser URLs again.
- ChatGPT ID tokens use `jose` RS256/JWKS issuer, audience, expiration, nonce and
  selected-account verification. Subscription account routing comes from its
  provider-issued access token. Decoding a routing/expiry claim is not treated
  as proof of identity.
- Credentials default to OS app config `dev.simrep.app/ai`; Tauri supplies its
  own app-config path. `SIMREP_AI_CONFIG_DIR` is runtime configuration, never a
  WebView parameter. Repository paths and symlinked config directories fail.
  Atomic file replacement uses `0600`, directories `0700`. Windows relies on
  the per-user app-data ACL; Unix chmod is not a Windows ACL API.
- ChatGPT uses `account-<hash>.json`, `registration-<hash>.json`, `host.json`;
  other flows use `provider-<id>.json`. Keys minted by OAuth are credentials.
  No provider token/error body/stderr is logged or relayed.
- Refresh and store mutations serialize in-process and hold an exclusive
  cross-process `session.lock`, protecting rotating refresh tokens. After a
  crash remove a stale lock only after all owners have stopped. Locks are never
  stolen. Temporary refresh failures retain credentials; terminal ones delete
  unusable credentials and expose stable `AUTH_EXPIRED`.
- Claude/Codex built-in shell/file/browser tools and external settings are
  disabled. Each session accepts server-side harness system instructions.
  Generic, native Devin and Kiro tool calls all execute through the existing
  MCP proxy. Mutations need UI approval; denial, timeout and cancellation fail
  closed. Workspaces containing known credential stores are refused.
- Sign-out cancels authorization before deleting tokens. It does not assert
  remote revocation for providers without a confirmed revocation operation.
  Permanent OpenRouter keys can additionally be revoked in the provider account.

로그인 정보는 앱 설정 폴더에만 보관하며 갱신 토큰 교체는 잠금으로 직렬화합니다.
WebView에는 표시용 계정 정보와 기기 코드만 전달합니다. 인증 URL은 Rust만 열고,
로그아웃은 진행 중인 로그인이 자격 증명을 다시 저장하지 못하도록 먼저 취소합니다.
권한 승인 없이 파일을 바꾸는 MCP 호출은 실행하지 않습니다.

## Verification / 검증

```sh
cd ai
npm test
npm run typecheck
npm run licenses
# Only when explicitly requested; real requests may consume quota:
SIMREP_AI_REAL_SMOKE=claude npm run smoke:real
# Any tool-capable provider ID is accepted instead of claude.
cd ../app
npm test
npm run typecheck
npm run e2e -- e2e/ai.spec.ts
cd ..
CARGO_TARGET_DIR=/tmp/simrep-provider-tauri cargo test --manifest-path app/src-tauri/Cargo.toml
CARGO_TARGET_DIR=/tmp/simrep-provider-tauri cargo clippy --manifest-path app/src-tauri/Cargo.toml --all-targets -- -D warnings
SIMREP_BIN=/absolute/path/to/simrep node mcp/smoke.mjs
```

Ordinary auth tests use local fake servers, real loopback callbacks, local JWKS
and an injected device-poll clock (no sleeps/network/accounts). They cover every
implemented provider login, refresh rotation, cancellation and storage.
Each generic API-key route drives one real MCP tool through a local fake wire
server; mutation denial and path traversal are also exercised. Kiro tests use
CRC-checked AWS eventstream frames and a real MCP call. Claude/Devin use fake SDK
or native-adapter output with the real MCP proxy; Codex's pinned native binary
also initializes its restricted catalog/MCP locally.

Real inference, billing, interactive consent, entitlement, packaged native
binaries and platform browser/ACL behavior are **not** established by fake-server
tests. Real smokes skip unless `SIMREP_AI_REAL_SMOKE=<provider>` is explicitly
set. Provider and harness changes must be integrated together before claiming a
real provider-plus-harness smoke.

## Licences and source provenance / 라이선스

The installed alias `@earendil-works/pi-ai` identifies itself as
`@code-yeongyu/senpi-ai` **2026.10.25-2**, `license: MIT`.
Its README `## License` section contains **only `MIT`**, not a copyright line,
and its published package has no LICENSE file. The canonical
[senpi root LICENSE](https://github.com/code-yeongyu/senpi/blob/main/LICENSE)
was therefore also read: `Copyright (c) 2025 Mario Zechner (upstream pi-mono)`
and `Copyright (c) 2026 Yeongyu Kim and senpi contributors`.
The [pi-mono root](https://github.com/badlogic/pi-mono/blob/main/LICENSE)
currently says `Copyright (c) 2025 Mario Zechner`. These are the inspected lines,
not an inferred 2026 Mario notice.

Kiro sources are pinned to MIT
[omo-accounts 3df128a](https://github.com/eddieparc/omo-accounts/tree/3df128a51458c829e61a4da62d6545ed55bcc1c6/src/providers/kiro),
`Copyright (c) 2026 eddieparc`. Every adapted file carries its permission/warranty
notice, source/version and modification notice. Full notices and the exact file
list are in [licenses/THIRD_PARTY.md](../licenses/THIRD_PARTY.md).
No Sustainable Use License code from omo's top-level plugin was copied.

| Direct package | Exact version | Licence / terms |
| --- | --- | --- |
| `@anthropic-ai/claude-agent-sdk` | 0.3.311 | Anthropic commercial terms, not an OSI licence |
| `@openai/codex` | 0.164.0 | Apache-2.0 |
| `@code-yeongyu/senpi-ai` | 2026.10.25-2 | MIT |
| `jose` | 5.10.0 | MIT |
| `typescript` (development) | 5.9.3 | Apache-2.0 |
| `@types/node` (development) | 18.19.130 | MIT |

Transitive versions are locked in `package-lock.json`. Preserve redistributed
`node_modules` notices, including `@bufbuild/protobuf`'s Apache-2.0 AND BSD-3-Clause
notice and the Anthropic SDK/native commercial terms. The MIT auth adaptation
does not relicense either the SDK or the existing Prosperity-licensed MCP files.
