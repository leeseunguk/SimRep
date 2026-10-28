# SimRep AI bridge protocol v1

`node ai/server.mjs` uses UTF-8 newline-delimited JSON-RPC 2.0 on stdin/stdout.
Stdout contains protocol messages only. IDs are strings or integers; overlapping
requests receive their own ID back. These additions preserve the original v1
methods and account fields.

## Desktop transport

Invoke Tauri `ai_request` with `{request: {jsonrpc: "2.0", id, method, params}}`.
It returns the complete response object. Register `ai-event` **before** sending
requests; its payload is the notification's `params`. Rust owns and terminates
the sidecar/process group. Node.js 24+ is required.

## Requests

| Method | Params | Result |
| --- | --- | --- |
| `auth.providers` | `{}` | `{providers: ProviderMetadata[]}` |
| `auth.status` | `{provider?}` | `{providers: ProviderStatus[]}` |
| `auth.accounts` | `{provider?}` | `{accounts: Account[]}` |
| `auth.login` | `{provider, account?, region?}` | `{started: true}` |
| `auth.logout` | `{provider, account?}` | `{signedOut: true, remoteRevocationConfirmed: boolean}` |
| `auth.openUsage` | `{provider}` | `{opened: true}` |
| `chat.start` | `{provider, model?, workspace, account?, language?}` | `{sessionId, provider, model, workspace}` |
| `chat.send` | `{sessionId, text}` | `{turnId}` |
| `chat.cancel` | `{sessionId}` | `{cancelled: true}` |
| `approval.resolve` | `{approvalId, allow: boolean}` | `{resolved: true}` |

`auth.providers` is the provider registry, not an entitlement check. Each entry
has `{id, label, authKind, planNote?, models?, toolCalling, withoutPlan,
planEnabledMeaning, apiKeyEnv?, usageUrl?, regions?, signInSupported}`.
`authKind` is `oauth | cli-login | device-code | api-key`. `models` are string
IDs; `toolCalling: false` plus `unsupportedReason` means sign-in only, and
`chat.start` fails with `PROVIDER_UNSUPPORTED`. The registry includes ChatGPT,
Claude, Copilot, xAI, OpenRouter, Kimi Code, Devin, Meta, Kiro, Cursor, Radius,
and the environment-key providers listed in README.md.

`ProviderStatus` is `{provider, authenticated, accounts: Account[], models:
string[], toolCalling: boolean, message?}`. Older clients may ignore the added
fields. Copilot models are the account's enabled, tool-capable Chat Completions
models, not the provider's whole catalog. Other lists are conservative defaults,
not a claim that every signed-in account can use them.

`Account` remains `{id, provider, label, email?, authenticated, planEnabled}`.
No access token, refresh token, ID token, raw token response, key or authorization
URL is returned. IDs are opaque and stable. The first authenticated account is
the default unless `chat.start.account` is supplied.

### Login, key and plan semantics

- `auth.login` starts an asynchronous attempt. Completion/failure is an
  `auth`/`error` event. `account` reauthorizes a saved ChatGPT registration;
  other providers currently have one local account each.
- ChatGPT defaults to Codex subscription PKCE (same maintained client as omo).
  `SIMREP_CHATGPT_AUTH_FLOW=siwc` selects the previous dynamic-client SIWC route.
  Its registrations remain separate from the subscription client's registrations.
- Claude uses bridge-owned Anthropic PKCE and passes the selected credential to
  the Agent SDK. It never calls `claude auth login`, reads a CLI credential, or
  signs the host CLI out.
- `region` is accepted for `kimi-coding`: `mainland-cn` or `global`, defaulting to
  `KIMI_CODE_REGION` or `mainland-cn`. Region is persisted; subsequent refresh and
  inference follow the saved region, not later environment changes.
- API keys are **environment-only**. There is no key-entry control and no
  `auth.setApiKey` method. OAuth OpenRouter/Meta keys are stored by the auth flow,
  not submitted through the WebView. An environment key takes precedence over
  the same provider's OAuth credential.
- `planEnabledMeaning` defines the flag per provider. ChatGPT: subscription
  credential available, or the required direct-use grant in optional SIWC mode.
  Claude: bridge OAuth selected; false on an authenticated account means API
  key billing. OpenRouter: OAuth-minted key available, billed from credits.
  Copilot, xAI, Kimi, Devin, Meta, Kiro, Cursor and Radius: provider sign-in
  credential available, not proof of free usage, model access or a plan allowance.
  Environment-key providers always report false. `withoutPlan: blocked` prevents
  interpreting a missing required grant as an API key; `apiKey` identifies the
  environment-key alternative. Billing distinctions are in README.md.
- Logout cancels in-flight authorization before deleting credentials. ChatGPT
  retains non-secret client/account registration and attempts remote revocation.
  Other providers return `remoteRevocationConfirmed: false`: local deletion is
  not a claim that the provider revoked a session or permanent minted key.
  An inherited environment key cannot be deleted by logout (`ENV_API_KEY`).

`auth.openUsage` accepts a provider ID, **never an arbitrary URL**. It asks Rust
to open that provider's fixed usage page (currently ChatGPT and Claude). The
WebView must not use an opener plugin, `window.open`, or a target-blank anchor.

`workspace` must be an existing absolute directory which does not contain a
known AI credential store. Every chat uses only the workspace-restricted SimRep
MCP approval proxy. No shell/file provider tools are offered. All session
implementations accept a server-side `systemPrompt` option for the harness;
it is not a WebView-controlled `chat.start` parameter. One turn per session runs
at a time. `language` is an optional 2-35 character language tag. The harness
prepares provider input and verifies tool-derived claims before releasing the
answer; its MCP recipe/workflow tools use the same approval proxy.

## Notifications

`{"jsonrpc":"2.0","method":"event","params":{...}}`

Every event has `type` and `provider`. Chat events also carry `sessionId` and
`turnId`. They may additionally carry `activeSkill`: `ion-optics`, `einzel`,
`esa`, `quadrupole`, `magnetic`, or `cad`. The UI displays the active recipe
badge. Model/tool text is untrusted text, never trusted HTML.

If `language` is omitted the harness matches the user message. Every provider
receives the same SimRep harness system prompt.
See [harness/README.md](harness/README.md) for workflow tools,
verification, output handles and mock/real-provider evals.

| type | Fields |
| --- | --- |
| `auth` | `status: ProviderStatus` |
| `device_code` | `userCode, verificationUri, expiresIn` (seconds) |
| `delta` | `text` (append to assistant message) |
| `tool_call` | `callId, name, arguments` |
| `tool_result` | `callId, name?, result` |
| `approval` | `approvalId, name, arguments` |
| `done` | `status: "completed" \| "interrupted" \| "failed"` |
| `error` | `code, message, details?` (safe diagnostics, never credentials) |

The device user code is public display information, not the private device code
sent to the token endpoint. Display it until `auth`/`error`, with the supplied
lifetime. `verificationUri` is display-only: Rust opens the separately validated
host URL, which may include the public user code. No token or authorization URL
is included in the event.

Every mutating MCP tool requires approval before the real MCP server sees it.
Denial, timeout (two minutes) and cancellation fail closed. Each accepted
`chat.send` ends with exactly one `done`; `error` may precede it.

## Host-only notification

`{"jsonrpc":"2.0","method":"host.open_url","params":{"url":"..."}}`

Rust alone validates fixed first-party HTTPS origins/routes and opens the system
browser without a shell. This notification must never be logged or forwarded to
the WebView: a returning SIWC URL can contain an ID-token hint.

## Errors

JSON-RPC numeric codes: `-32700` parse, `-32600` invalid request, `-32601` unknown
method, `-32602` invalid params, `-32000` operation failed. `error.data.code` and
notification `code` use these stable bridge codes:

| Code | Meaning / recovery |
| --- | --- |
| `INVALID_REQUEST` | Invalid JSON-RPC envelope. |
| `INVALID_PARAMS` | Unknown provider/account, invalid region, workspace, session, model or decision. |
| `METHOD_NOT_FOUND` | Unsupported method; update the client/bridge together. |
| `WORKSPACE_UNSAFE` | Workspace contains a protected AI credential directory; choose a project directory. |
| `CONFIG_UNSAFE` | Config is in the repository, a symlink, or not owner-only; correct the directory/permissions. |
| `BUSY` | Login/turn already running, or cross-process credential lock is held. |
| `CANCELLED` | Attempt/turn was cancelled. |
| `AUTH_REQUIRED` | No selected provider credential; sign in or configure its key. |
| `AUTH_EXPIRED` | Credential cannot be refreshed, was rejected with HTTP 401, or its grant was invalidated; sign in again. Terminal refresh deletes unusable local credentials. |
| `PLAN_PERMISSION_REQUIRED` | Optional ChatGPT SIWC route lacks the direct-use grant; reauthorize. |
| `AUTH_DENIED` | Browser/device authorization was declined. |
| `AUTH_TIMEOUT` | Browser/device authorization expired. |
| `AUTH_CALLBACK` | Callback host/path/state/code/one-use check failed, or required loopback ports are unavailable. |
| `AUTH_IDENTITY` | ChatGPT ID-token signature/issuer/audience/nonce/account verification failed. |
| `AUTH_RESPONSE` | Malformed token/device response or untrusted verification/API endpoint. |
| `AUTH_HTTP` | Authorization network/HTTP failure; temporary refresh failures retain credentials. |
| `AUTH_FAILED` | Login did not complete (legacy clients may still emit this). |
| `ENV_API_KEY` | Environment-only key cannot be entered/deleted by the bridge; change the app environment and restart. |
| `CLAUDE_CLI_MISSING` | Legacy v1 CLI-login error; bridge PKCE no longer emits it. |
| `TOOL_DENIED` | Unregistered tool or invalid tool arguments. A user denial normally returns an MCP `isError` result instead. |
| `PROVIDER_UNSUPPORTED` | Sign-in-only provider or no enabled tool-capable model. |
| `PROVIDER_RESPONSE` | Malformed/truncated wire response, tool arguments or event-stream frame. |
| `PROVIDER_TOOL_LIMIT` | More than 32 tool-call rounds in one turn. |
| `PROVIDER_ERROR` | Sanitized upstream inference/installation/connectivity failure. |
| `PROVIDER_CLOSED` | SDK/MCP process disconnected. |
| `PROVIDER_TIMEOUT` | SDK/MCP RPC deadline exceeded. |
| `PROVIDER_RPC` | SDK/MCP rejected an RPC operation. |

`BRIDGE_UNAVAILABLE` is UI-local (Tauri invocation/sidecar startup failed), not a
JSON-RPC bridge code. Raw OAuth names such as `invalid_grant` are private
implementation details, never public error codes. Clients may retain legacy
refresh-code handling for older bridges. No raw provider stderr, headers,
environment, token response or arbitrary exception text crosses the protocol.
