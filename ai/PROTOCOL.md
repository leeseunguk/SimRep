# SimRep AI bridge protocol v1

The Node sidecar (`node ai/server.mjs`) uses UTF-8 newline-delimited JSON-RPC 2.0
on stdin/stdout. Stdout contains protocol messages only. Request IDs are strings
or integers. Requests may overlap; each response echoes its ID. Methods below
are stable for the desktop UI. Providers are `chatgpt` and `claude`.

## Desktop transport

Invoke Tauri command `ai_request` with `{request: {jsonrpc: "2.0", id, method,
params: {...}}}`. It returns the JSON-RPC response object, including `error`
on a protocol or provider failure. Listen to Tauri event `ai-event` **before**
starting a request. Its payload is the notification `params` described below.
The Rust host owns one sidecar and terminates it when the app exits.

## Requests

| Method | Params | Result |
| --- | --- | --- |
| `auth.status` | `{provider?}` | `{providers: ProviderStatus[]}` |
| `auth.accounts` | `{provider?}` | `{accounts: Account[]}` |
| `auth.login` | `{provider, account?}` | `{started: true}` |
| `auth.logout` | `{provider, account?}` | `{signedOut: true, remoteRevocationConfirmed: boolean}` |
| `chat.start` | `{provider, model?, workspace, account?}` | `{sessionId, provider, model, workspace}` |
| `chat.send` | `{sessionId, text}` | `{turnId}` |
| `chat.cancel` | `{sessionId}` | `{cancelled: true}` |
| `approval.resolve` | `{approvalId, allow: boolean}` | `{resolved: true}` |

`auth.login` starts an asynchronous login. Completion/failure arrives via
`auth`/`error` events. A ChatGPT `account` reauthorizes that saved registration;
omitting it registers another account. Claude uses the official CLI's shared
login and logout; an inherited `ANTHROPIC_API_KEY` takes precedence and cannot
be deleted by logout. No method accepts or returns tokens or API keys.

`ProviderStatus` is `{provider, authenticated, accounts: Account[], message?}`.
`Account` is `{id, provider, label, email?, authenticated, planEnabled}`. IDs
are opaque and stable. Saved ChatGPT registrations remain visible after logout,
but their credential record is deleted. `chat.start` must select an authenticated
account (explicit `account`, or the first authenticated account). A missing
ChatGPT direct-use grant prevents inference, even with a valid identity.
`model` is a provider model ID or alias; omission uses the provider default.
`workspace` must be an existing absolute directory. Each chat gets only the
SimRep MCP server restricted to that workspace; built-in shell/file tools are
disabled. One turn per session can run at a time.

## Notifications

`{"jsonrpc":"2.0","method":"event","params":{...}}`

Every event has `type` and `provider`; chat events also have `sessionId` and
`turnId`. Event-specific fields:

| type | Fields |
| --- | --- |
| `auth` | `status: ProviderStatus` |
| `delta` | `text` (append to assistant message) |
| `tool_call` | `callId, name, arguments` |
| `tool_result` | `callId, name?, result` |
| `approval` | `approvalId, name, arguments` |
| `done` | `status: "completed" \| "interrupted" \| "failed"` |
| `error` | `code, message, details?` (safe diagnostics; never credentials) |

Mutating MCP tools require an `approval` response. Read-only inspection tools
may run without prompting. Denying/ignoring approval never permits execution;
pending approvals expire after two minutes and cancellation denies them.
Each accepted `chat.send` ends with exactly one `done`; errors may precede it.

## Host-only notification

`{"jsonrpc":"2.0","method":"host.open_url","params":{"url":"..."}}`
is consumed **only by Rust**, which validates the HTTPS authorization origin and
opens the system browser. It must never be forwarded to the WebView or logged:
reauthorization URLs may contain an ID-token hint. Claude's own CLI owns its
browser login. The WebView sees only account display information.

## Errors

JSON-RPC errors use `-32700` (parse), `-32600` (invalid request), `-32601`
(unknown method), `-32602` (invalid params), or `-32000` (operation failed).
`error.data.code` is a stable bridge code such as `AUTH_REQUIRED`,
`PLAN_PERMISSION_REQUIRED`, `BUSY`, or `PROVIDER_ERROR`; `data.details`, when
present, is sanitized provider metadata (HTTP status, code, request ID).
No raw provider stderr, environment, token response, or authorization URL is
exposed. Do not render model/tool text as trusted HTML.
