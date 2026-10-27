import path from 'node:path'
import { mkdir } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import { RpcProcess } from './rpc.mjs'
import { HERE } from './tools.mjs'
import { BridgeError, providerDiagnostic } from './errors.mjs'

export class ChatGPTSession {
  constructor({ auth, account, workspace, model, gate, emit, rpcFactory = (command, args, options) => new RpcProcess(command, args, options) }) {
    this.auth = auth; this.account = account; this.workspace = workspace; this.model = model
    this.gate = gate; this.emit = emit; this.rpcFactory = rpcFactory
    this.rpc = undefined; this.threadId = undefined; this.token = undefined; this.turnId = undefined
    this.completion = undefined
    this.catalog = 'catalog-' + randomUUID()
    this.cancelled = false
    this.secrets = []
  }

  async connect() {
    const record = await this.auth.access(this.account)
    if (this.cancelled) throw new BridgeError('CANCELLED', 'Turn cancelled.')
    if (this.rpc && this.token === record.access_token && !this.rpc.closed) return
    this.rpc?.close()
    // An app-owned Codex home avoids importing the user's unrelated MCP servers
    // and stores resumable threads separately from OAuth credential records.
    const home = path.join(this.auth.store.directory, 'codex')
    await mkdir(home, { recursive: true, mode: 0o700 })
    // A conservative catalog removes apply_patch as well as shell capability.
    // Merely disabling apply_patch_freeform is insufficient: Codex can otherwise
    // expose its function-form file editor from bundled model metadata.
    await this.auth.store.write(this.catalog, { models: [{
      slug: this.model, display_name: this.model, description: 'SimRep MCP agent',
      supported_reasoning_levels: [], shell_type: 'disabled', visibility: 'list',
      supported_in_api: true, priority: 0, availability_nux: null, upgrade: null,
      model_messages: { instructions_template: 'Operate the SimRep ion optics simulator using only its supplied MCP tools.' },
      support_verbosity: false, default_verbosity: null, apply_patch_tool_type: null,
      truncation_policy: { mode: 'tokens', limit: 10_000 },
      experimental_supported_tools: [], input_modalities: ['text'],
      include_skills_usage_instructions: false, include_apps_usage_instructions: false,
      include_plugin_usage_instructions: false, node_repl_disabled: true,
    }] })
    if (this.cancelled) throw new BridgeError('CANCELLED', 'Turn cancelled.')
    const overrides = {
      model_provider: 'openai_chatgpt_plan',
      model_catalog_json: path.join(this.auth.store.directory, this.catalog + '.json'),
      'model_providers.openai_chatgpt_plan.name': 'ChatGPT plan',
      'model_providers.openai_chatgpt_plan.base_url': 'https://api.openai.com/v1',
      'model_providers.openai_chatgpt_plan.env_key': 'ACCESS_TOKEN',
      'model_providers.openai_chatgpt_plan.wire_api': 'responses',
      'model_providers.openai_chatgpt_plan.requires_openai_auth': false,
      'model_providers.openai_chatgpt_plan.supports_websockets': false,
      'features.shell_tool': false, 'features.unified_exec': false,
      'features.apply_patch_freeform': false, 'features.js_repl': false,
      'features.code_mode': false, 'features.multi_agent': false,
      'features.multi_agent_v2': false, 'features.apps': false, 'features.plugins': false,
      'features.browser_use': false, 'features.computer_use': false, 'features.view_image': false,
      'features.image_generation': false, 'features.hooks': false,
      'features.codex_hooks': false, 'features.skip_host_skill_discovery': true,
      web_search: 'disabled', approval_policy: 'never', sandbox_mode: 'read-only',
      'tools.update_plan.enabled': false,
      'tools.experimental_request_user_input.enabled': false,
      mcp_servers: { simrep: { ...this.gate.config, tool_timeout_sec: 3600 } },
    }
    // JSON objects are not TOML inline tables. Encode the fixed config map into
    // dotted leaf overrides; only primitive values and arrays are serialized.
    const leaves = []
    const flatten = (object, prefix = '') => {
      for (const [key, value] of Object.entries(object)) {
        const name = prefix ? `${prefix}.${key}` : key
        if (value && typeof value === 'object' && !Array.isArray(value)) flatten(value, name)
        else leaves.push('-c', `${name}=${JSON.stringify(value)}`)
      }
    }
    flatten(overrides)
    const executable = path.join(HERE, 'node_modules', '@openai', 'codex', 'bin', 'codex.js')
    const rpc = this.rpcFactory(process.execPath, [executable, 'app-server', '--listen', 'stdio://', ...leaves], {
      cwd: home, env: { ...process.env, CODEX_HOME: home, ACCESS_TOKEN: record.access_token },
    })
    this.rpc = rpc
    this.token = record.access_token
    this.secrets = [record.access_token, record.refresh_token, record.id_token, process.env.ANTHROPIC_API_KEY]
    rpc.onmessage = async message => {
      const p = message.params
      if (message.id !== undefined && message.method) {
        // Never approve shell, filesystem, elicitation or unknown server requests.
        rpc.send({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'Only SimRep MCP tools are permitted.' } })
      } else if (p?.threadId === this.threadId && message.method === 'item/agentMessage/delta') {
        this.emit({ type: 'delta', text: p.delta })
      } else if (p?.threadId === this.threadId && message.method === 'turn/completed') {
        const status = p.turn.status
        if (status === 'failed') this.emit({ type: 'error', ...providerDiagnostic(p.turn.error, this.secrets) })
        this.completion?.resolve(['completed', 'interrupted'].includes(status) ? status : 'failed')
      }
    }
    rpc.onclose = () => this.completion?.reject(new BridgeError('PROVIDER_CLOSED', 'ChatGPT process disconnected.'))
    await rpc.request('initialize', { clientInfo: { name: 'SimRep', title: 'SimRep', version: '0.1.0' } })
    rpc.send({ jsonrpc: '2.0', method: 'initialized', params: {} })
    // Do not load project-local .codex configuration (which can launch other
    // MCP executables). The simulator workspace is conveyed only to our proxy.
    const params = { model: this.model, cwd: home, sandbox: 'read-only', approvalPolicy: 'never',
      baseInstructions: 'Use only the SimRep MCP tools to operate the ion optics simulator. Units are mm, us, V, gauss, amu and eV.' }
    const result = await rpc.request(this.threadId ? 'thread/resume' : 'thread/start',
      this.threadId ? { ...params, threadId: this.threadId } : params)
    this.threadId = result.thread.id
  }

  async send(text) {
    this.cancelled = false
    await this.connect()
    if (this.cancelled) return 'interrupted'
    const completed = new Promise((resolve, reject) => { this.completion = { resolve, reject } })
    // Subscribe before turn/start because a short turn may finish before its
    // request response. Attach rejection handling immediately for process exit.
    completed.catch(() => {})
    try {
      const result = await this.rpc.request('turn/start', { threadId: this.threadId, input: [{ type: 'text', text, text_elements: [] }] })
      this.turnId = result.turn.id
      return await completed
    } finally { this.completion = undefined; this.turnId = undefined }
  }

  async cancel() {
    this.cancelled = true
    try {
      if (this.turnId && this.rpc) await this.rpc.request('turn/interrupt', { threadId: this.threadId, turnId: this.turnId }, 5000)
    } finally {
      // Closing the provider also closes its proxy stdin, terminating a running
      // simulator child even when the MCP server ignores cancellation notices.
      this.rpc?.close()
    }
  }

  close() {
    this.rpc?.close(); this.gate.close()
    this.token = undefined; this.secrets = []
    void this.auth.store.remove(this.catalog)
  }
}
