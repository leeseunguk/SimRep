import { query } from '@anthropic-ai/claude-agent-sdk'
import path from 'node:path'
import { BridgeError } from './errors.mjs'
import { READ_ONLY, TOOL_NAMES } from './tools.mjs'
import { ProviderAuth } from './auth/providers.mjs'
import { Store } from './store.mjs'

// Bridge-owned PKCE and refresh. The SDK never reads the host CLI login.
export class ClaudeAuth extends ProviderAuth {
  constructor(options = {}) { super('claude', options) }
}

export class ClaudeSession {
  constructor({ auth, account, workspace, model, gate, emit, systemPrompt = undefined, queryFn = query }) {
    this.auth = auth; this.account = account
    this.workspace = workspace; this.model = model; this.gate = gate; this.emit = emit; this.queryFn = queryFn
    this.systemPrompt = systemPrompt
    this.resume = undefined; this.client = undefined; this.controller = undefined
  }

  async send(text) {
    this.controller = new AbortController()
    const credential = await this.auth.access(this.account)
    const config = new Store(path.join(this.auth.store.directory, 'claude-sdk'))
    await config.init()
    const env = Object.fromEntries(['PATH', 'HOME', 'USERPROFILE', 'SystemRoot', 'WINDIR', 'COMSPEC', 'APPDATA',
      'TMPDIR', 'TEMP', 'LANG', 'HTTPS_PROXY', 'HTTP_PROXY', 'NO_PROXY', 'NODE_EXTRA_CA_CERTS']
      .filter(k => process.env[k]).map(k => [k, process.env[k]]))
    for (const name of ['ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'ANTHROPIC_BASE_URL', 'CLAUDE_CODE_OAUTH_TOKEN',
      'CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR', 'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY']) env[name] = undefined
    env.CLAUDE_CONFIG_DIR = config.directory
    if (credential.apiKey) env.ANTHROPIC_API_KEY = credential.apiKey
    else env.CLAUDE_CODE_OAUTH_TOKEN = credential.access_token
    if (this.controller.signal.aborted) return 'interrupted'
    const client = this.queryFn({
      prompt: text,
      options: {
        cwd: this.workspace, model: this.model, resume: this.resume,
        env,
        abortController: this.controller, includePartialMessages: true,
        tools: [], allowedTools: READ_ONLY.map(n => `mcp__simrep__${n}`),
        permissionMode: 'default', settingSources: [], strictMcpConfig: true,
        mcpServers: { simrep: this.gate.config },
        canUseTool: async (name, input) => TOOL_NAMES.some(n => name === `mcp__simrep__${n}`)
          // The MCP proxy owns mandatory approval for every mutation.
          ? { behavior: 'allow', updatedInput: input } : { behavior: 'deny', message: 'Only workspace-restricted SimRep tools are available.' },
        systemPrompt: this.systemPrompt || 'You operate the SimRep ion optics simulator. Use only the provided SimRep MCP tools. Units are mm, us, V, gauss, amu and eV. Ask before changing the project.',
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
