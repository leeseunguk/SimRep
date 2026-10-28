import { randomUUID } from 'node:crypto'
import { realpath, stat } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { ChatGPTAuth } from './oauth.mjs'
import { ClaudeAuth, ClaudeSession } from './claude.mjs'
import { ChatGPTSession } from './chatgpt.mjs'
import { toolGate } from './tools.mjs'
import { BridgeError, publicError } from './errors.mjs'
import { createProviderAuth } from './auth/providers.mjs'
import { KiroAuth } from './auth/kiro.mjs'
import { ProviderSession } from './providers/client.mjs'
import { PROVIDERS, providerById, providerMetadata } from './providers/table.mjs'
import { createHarness } from './harness/index.mjs'

function required(value, name) {
  if (typeof value !== 'string' || !value.trim()) throw new BridgeError('INVALID_PARAMS', `${name} must be a non-empty string.`)
  return value
}
function provider(value) {
  if (!providerById(value)) throw new BridgeError('INVALID_PARAMS', 'Unknown provider.')
  return value
}

export class Bridge {
  constructor({ notify = (_message) => {}, chatgpt = new ChatGPTAuth({ flow: process.env.SIMREP_CHATGPT_AUTH_FLOW || 'subscription' }), claude = new ClaudeAuth(),
    auth = {}, createProvider = options => new ProviderSession(options),
    createChatGPT = options => new ChatGPTSession(options), createClaude = options => new ClaudeSession(options) } = {}) {
    this.notify = notify
    this.auth = Object.fromEntries(PROVIDERS.map(p => [p.id, createProviderAuth(p.id)]))
    Object.assign(this.auth, { chatgpt, claude, kiro: new KiroAuth() }, auth)
    this.create = Object.fromEntries(PROVIDERS.map(p => [p.id, options => createProvider({ ...options, provider: p })]))
    Object.assign(this.create, { chatgpt: createChatGPT, claude: createClaude })
    this.sessions = new Map(); this.approvals = new Map()
  }

  event(provider, fields) { this.notify({ jsonrpc: '2.0', method: 'event', params: { provider, ...fields } }) }
  async status(p) {
    const accounts = await this.auth[p].accounts()
    const info = providerById(p)
    return { provider: p, authenticated: accounts.some(a => a.authenticated), accounts,
      models: this.auth[p].models ? await this.auth[p].models() : info.models,
      toolCalling: info.toolCalling, ...(info.unsupportedReason && { message: info.unsupportedReason }) }
  }

  approve(session, name, args) {
    const approvalId = randomUUID()
    return new Promise(resolve => {
      const settle = allow => { clearTimeout(timer); this.approvals.delete(approvalId); resolve(allow) }
      const expiresAt = Date.now() + 120_000
      const timer = setTimeout(() => settle(false), Math.max(0, expiresAt - Date.now()))
      this.approvals.set(approvalId, { sessionId: session.id, settle })
      this.emitChat(session, { type: 'approval', approvalId, name, arguments: args, expiresAt })
    })
  }

  emitChat(session, fields) {
    this.event(session.provider, { sessionId: session.id, turnId: session.turn?.id, ...fields })
  }
  async stop(session) {
    if (session.turn) session.turn.cancelled = true
    for (const entry of this.approvals.values()) if (entry.sessionId === session.id) entry.settle(false)
    await session.client.cancel()
  }

  /** @param {string} method
   * @param {{provider?: string, account?: string, region?: string, workspace?: string, model?: string, language?: string, sessionId?: string, text?: string, approvalId?: string, allow?: boolean}} params */
  async dispatch(method, params = {}) {
    if (!params || typeof params !== 'object' || Array.isArray(params)) throw new BridgeError('INVALID_PARAMS', 'params must be an object.')
    switch (method) {
      case 'auth.providers': return { providers: PROVIDERS.map(providerMetadata) }
      case 'auth.openUsage': {
        const p = providerById(provider(params.provider))
        if (!p.usageUrl) throw new BridgeError('INVALID_PARAMS', 'No usage page is configured for this provider.')
        this.notify({ jsonrpc: '2.0', method: 'host.open_url', params: { url: p.usageUrl } })
        return { opened: true }
      }
      case 'auth.status':
      case 'auth.accounts': {
        const providers = params.provider ? [provider(params.provider)] : PROVIDERS.map(p => p.id)
        const statuses = await Promise.all(providers.map(p => this.status(p)))
        return method === 'auth.status' ? { providers: statuses } : { accounts: statuses.flatMap(s => s.accounts) }
      }
      case 'auth.login': {
        const p = provider(params.provider)
        const login = await this.auth[p].begin(params.account, { region: params.region })
        if (login.url) this.notify({ jsonrpc: '2.0', method: 'host.open_url', params: { url: login.url } })
        if (login.deviceCode) this.event(p, { type: 'device_code', ...login.deviceCode })
        void login.completed.then(async () => this.event(p, { type: 'auth', status: await this.status(p) }))
          .catch(error => this.event(p, { type: 'error', ...publicError(error) }))
        return { started: true }
      }
      case 'auth.logout': {
        const p = provider(params.provider)
        const accounts = await this.auth[p].accounts()
        const account = params.account || accounts[0]?.id
        if (!account && p === 'chatgpt') {
          this.auth.chatgpt.close()
          return { signedOut: true, remoteRevocationConfirmed: true }
        }
        for (const session of this.sessions.values()) if (session.provider === p && (p !== 'chatgpt' || session.account === account)) {
          await this.stop(session); session.client.close(); this.sessions.delete(session.id)
        }
        const result = await this.auth[p].logout(account)
        this.event(p, { type: 'auth', status: await this.status(p) })
        return result
      }
      case 'chat.start': {
        const p = provider(params.provider)
        const info = providerById(p)
        if (!info.toolCalling) throw new BridgeError('PROVIDER_UNSUPPORTED', info.unsupportedReason)
        required(params.workspace, 'workspace')
        if (!path.isAbsolute(params.workspace)) throw new BridgeError('INVALID_PARAMS', 'workspace must be an absolute directory.')
        const workspace = await realpath(params.workspace)
        if (!(await stat(workspace)).isDirectory()) throw new BridgeError('INVALID_PARAMS', 'workspace must be a directory.')
        // A workspace may not encompass credential stores, even if chosen as
        // the home directory. Symlinks in MCP paths are independently resolved.
        const protectedDirs = [...Object.values(this.auth).map(a => a.store?.directory),
          ...['.claude', '.codex', '.omo', '.senpi'].map(d => path.join(os.homedir(), d))].filter(Boolean)
        for (const directory of protectedDirs) {
          const canonical = await realpath(directory).catch(() => path.resolve(directory))
          if (canonical === workspace || canonical.startsWith(workspace + path.sep)) throw new BridgeError('WORKSPACE_UNSAFE', 'Choose a project directory that does not contain AI credentials.')
        }
        const accounts = await this.auth[p].accounts()
        const account = accounts.find(a => params.account ? a.id === params.account && a.authenticated : a.authenticated)
        if (!account) throw new BridgeError('AUTH_REQUIRED', 'Sign in with the selected provider first.')
        if (p === 'chatgpt' && !account.planEnabled) throw new BridgeError('PLAN_PERMISSION_REQUIRED', 'Enable ChatGPT plan use before starting a chat.')
        if (params.model !== undefined) required(params.model, 'model')
        const models = this.auth[p].models ? await this.auth[p].models() : info.models
        const model = params.model || models[0] || info.defaultModel
        if (!model || (p === 'github-copilot' && !models.includes(model))) throw new BridgeError('PROVIDER_UNSUPPORTED', 'No enabled tool-capable model is available. Enable a model with the provider first.')
        const session = { id: randomUUID(), provider: p, account: account.id, model, workspace, turn: undefined, client: undefined, harness: undefined }
        if (params.language !== undefined && (typeof params.language !== 'string' || !/^[a-zA-Z-]{2,35}$/.test(params.language))) throw new BridgeError('INVALID_PARAMS', 'language must be a language tag.')
        const harness = await createHarness({ workspace, language: params.language, emit: fields => this.emitChat(session, fields) })
        const emit = fields => harness.emit(fields)
        const gate = await toolGate(workspace, (name, args) => this.approve(session, name, args), emit)
        try {
          session.client = this.create[p]({ auth: this.auth[p], account: account.id, workspace, model, gate, emit, systemPrompt: harness.systemPrompt })
          session.harness = harness
          this.sessions.set(session.id, session)
        } catch (error) { gate.close(); throw error }
        return { sessionId: session.id, provider: p, model, workspace }
      }
      case 'chat.send': {
        const session = this.sessions.get(required(params.sessionId, 'sessionId'))
        if (!session) throw new BridgeError('INVALID_PARAMS', 'Unknown chat session.')
        const text = required(params.text, 'text')
        if (text.length > 1_000_000) throw new BridgeError('INVALID_PARAMS', 'Message is too large.')
        if (session.turn) throw new BridgeError('BUSY', 'A turn is already running.')
        const turn = { id: randomUUID(), cancelled: false }
        session.turn = turn
        // Defer until the JSON-RPC acknowledgement is written; listeners are
        // still required before sending, since provider startup is asynchronous.
        setImmediate(() => {
          void (async () => {
            let status = 'failed'
            try {
              if (!turn.cancelled) {
                const prepared = await session.harness.prepare(text)
                if (!turn.cancelled) status = await session.client.send(prepared)
              }
            } catch (error) {
              if (!turn.cancelled) this.emitChat(session, { type: 'error', ...publicError(error) })
            } finally {
              for (const entry of this.approvals.values()) if (entry.sessionId === session.id) entry.settle(false)
              session.harness.finish(turn.cancelled ? 'interrupted' : status)
              session.harness.emit({ type: 'done', status: turn.cancelled ? 'interrupted' : status })
              session.turn = undefined
            }
          })()
        })
        return { turnId: turn.id }
      }
      case 'chat.cancel': {
        const session = this.sessions.get(required(params.sessionId, 'sessionId'))
        if (!session) throw new BridgeError('INVALID_PARAMS', 'Unknown chat session.')
        await this.stop(session)
        return { cancelled: true }
      }
      case 'approval.resolve': {
        const entry = this.approvals.get(required(params.approvalId, 'approvalId'))
        if (!entry || typeof params.allow !== 'boolean') throw new BridgeError('INVALID_PARAMS', 'Unknown approval or invalid decision.')
        entry.settle(params.allow)
        return { resolved: true }
      }
      default: throw new BridgeError('METHOD_NOT_FOUND', 'Unknown method.')
    }
  }

  async handle(request) {
    const id = typeof request?.id === 'string' || Number.isSafeInteger(request?.id) ? request.id : null
    try {
      if (id === null || request.jsonrpc !== '2.0' || typeof request.method !== 'string') throw new BridgeError('INVALID_REQUEST', 'Invalid JSON-RPC request.')
      return { jsonrpc: '2.0', id, result: await this.dispatch(request.method, request.params) }
    } catch (error) {
      const detail = publicError(error)
      const code = { INVALID_REQUEST: -32600, INVALID_PARAMS: -32602, METHOD_NOT_FOUND: -32601 }[detail.code] || -32000
      return { jsonrpc: '2.0', id, error: { code, message: detail.message, data: detail } }
    }
  }

  close() {
    for (const entry of this.approvals.values()) entry.settle(false)
    for (const session of this.sessions.values()) session.client.close()
    for (const auth of Object.values(this.auth)) auth.close()
  }
}
