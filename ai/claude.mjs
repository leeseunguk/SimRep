import { query } from '@anthropic-ai/claude-agent-sdk'
import { spawn } from 'node:child_process'
import { BridgeError } from './errors.mjs'
import { READ_ONLY, TOOL_NAMES } from './tools.mjs'

// Credential storage belongs exclusively to Claude Code. accountInfo performs
// the SDK initialize handshake without sending a user prompt or inference.
export class ClaudeAuth {
  constructor(queryFn = query) { this.queryFn = queryFn; this.loginChild = undefined }

  async accounts() {
    let client
    try {
      client = this.queryFn({ prompt: emptyInput(), options: { tools: [], settingSources: [], strictMcpConfig: true } })
      const info = await client.accountInfo()
      const hasSource = value => typeof value === 'string' && value.length > 0 && !['none', 'unknown'].includes(value.toLowerCase())
      const authenticated = Boolean(process.env.ANTHROPIC_API_KEY || hasSource(info.tokenSource) || hasSource(info.apiKeySource))
      return [{ id: 'claude-local', provider: 'claude', label: process.env.ANTHROPIC_API_KEY ? 'Anthropic API key (environment)' : info.email || 'Claude local login',
        ...(info.email ? { email: info.email } : {}), authenticated, planEnabled: authenticated && !process.env.ANTHROPIC_API_KEY }]
    } catch {
      return [{ id: 'claude-local', provider: 'claude', label: 'Claude local login', authenticated: false, planEnabled: false }]
    } finally { client?.close() }
  }

  login() {
    if (this.loginChild) throw new BridgeError('BUSY', 'Claude login is already running.')
    // Checked against the installed `claude auth --help`: login owns both the
    // browser flow and credentials. Its stdout/stderr are never relayed.
    const child = spawn(process.env.SIMREP_CLAUDE_BIN || 'claude', ['auth', 'login'], {
      shell: false, windowsHide: true, stdio: ['ignore', 'ignore', 'ignore'],
    })
    this.loginChild = child
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { child.kill(); reject(new BridgeError('AUTH_TIMEOUT', 'Claude login timed out. Run claude auth login in a terminal.')) }, 10 * 60_000)
      const finish = () => { clearTimeout(timer); this.loginChild = undefined }
      child.once('error', () => { finish(); reject(new BridgeError('CLAUDE_CLI_MISSING', 'Install Claude Code and run claude auth login.')) })
      child.once('exit', code => {
        finish()
        if (code === 0) resolve(undefined)
        else reject(new BridgeError('AUTH_FAILED', 'Claude login did not complete. Run claude auth login in a terminal.'))
      })
    })
  }

  async logout() {
    this.close()
    if (process.env.ANTHROPIC_API_KEY) throw new BridgeError('ENV_API_KEY', 'Remove ANTHROPIC_API_KEY from the app environment to sign out of API-key authentication.')
    await new Promise((resolve, reject) => {
      const child = spawn(process.env.SIMREP_CLAUDE_BIN || 'claude', ['auth', 'logout'], { shell: false, windowsHide: true, stdio: 'ignore' })
      child.once('error', () => reject(new BridgeError('CLAUDE_CLI_MISSING', 'Claude Code is not installed.')))
      child.once('exit', code => code === 0 ? resolve(undefined) : reject(new BridgeError('AUTH_FAILED', 'Claude logout failed.')))
    })
    return { signedOut: true, remoteRevocationConfirmed: false }
  }

  close() { this.loginChild?.kill(); this.loginChild = undefined }
}

async function* emptyInput() { /* Initialize only; never submit a prompt. */ }

export class ClaudeSession {
  constructor({ workspace, model, gate, emit, queryFn = query }) {
    this.workspace = workspace; this.model = model; this.gate = gate; this.emit = emit; this.queryFn = queryFn
    this.resume = undefined; this.client = undefined; this.controller = undefined
  }

  async send(text) {
    this.controller = new AbortController()
    const client = this.queryFn({
      prompt: text,
      options: {
        cwd: this.workspace, model: this.model, resume: this.resume,
        abortController: this.controller, includePartialMessages: true,
        tools: [], allowedTools: READ_ONLY.map(n => `mcp__simrep__${n}`),
        permissionMode: 'default', settingSources: [], strictMcpConfig: true,
        mcpServers: { simrep: this.gate.config },
        canUseTool: async (name, input) => TOOL_NAMES.some(n => name === `mcp__simrep__${n}`)
          // The MCP proxy owns mandatory approval for every mutation.
          ? { behavior: 'allow', updatedInput: input } : { behavior: 'deny', message: 'Only workspace-restricted SimRep tools are available.' },
        systemPrompt: 'You operate the SimRep ion optics simulator. Use only the provided SimRep MCP tools. Units are mm, us, V, gauss, amu and eV. Ask before changing the project.',
      },
    })
    this.client = client
    let success = false
    try {
      for await (const message of client) {
        if (message.type === 'system' && message.subtype === 'init') this.resume = message.session_id
        if (message.type === 'stream_event' && message.event.type === 'content_block_delta' && message.event.delta.type === 'text_delta') {
          this.emit({ type: 'delta', text: message.event.delta.text })
        }
        if (message.type === 'result') {
          success = !message.is_error
          if (!success) throw new BridgeError('PROVIDER_ERROR', 'Claude could not complete this turn. Check login and subscription limits.')
        }
      }
      return this.controller.signal.aborted ? 'interrupted' : success ? 'completed' : 'failed'
    } finally { client.close(); this.client = undefined }
  }

  cancel() { this.controller?.abort(); this.client?.close() }
  close() { this.cancel(); this.gate.close() }
}
