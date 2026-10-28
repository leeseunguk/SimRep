/*
 * Provider wire metadata and Devin context mapping adapted from
 * @code-yeongyu/senpi-ai 2026.10.25-2 dist/types.js, dist/providers/devin.js
 * and dist/api/devin-agent.js; modified for SimRep.
 * MIT License
 * Copyright (c) 2025 Mario Zechner (upstream pi-mono)
 * Copyright (c) 2026 Yeongyu Kim and senpi contributors
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
import { randomUUID } from 'node:crypto'
import { RpcProcess } from '../rpc.mjs'
import { TOOL_NAMES } from '../tools.mjs'
import { BridgeError } from '../errors.mjs'
import { providerById } from './table.mjs'
import { kiroRound } from './kiro.mjs'

const SYSTEM = 'You operate SimRep using only the provided workspace-restricted MCP tools. Units: mm, us, V, gauss, amu, eV. Tool mutations require user approval. Do not claim a tool succeeded when it returned an error.'
async function devinStream(model, context, options) {
  const { streamSimple } = await import('@code-yeongyu/senpi-ai/compat')
  const { devinProvider } = await import('@code-yeongyu/senpi-ai/providers/devin')
  const seed = devinProvider().getModels().find(m => m.id === model)
  if (!seed) throw new BridgeError('INVALID_PARAMS', 'Choose a supported Devin model.')
  return streamSimple(seed, context, options)
}

export class ProviderSession {
  constructor({ provider, auth, account, workspace, model, gate, emit, systemPrompt = undefined, fetcher = fetch, nativeStream = devinStream }) {
    this.provider = typeof provider === 'string' ? providerById(provider) : provider
    this.auth = auth; this.account = account; this.workspace = workspace; this.model = model
    this.gate = gate; this.emit = emit; this.fetcher = fetcher; this.nativeStream = nativeStream
    this.systemPrompt = systemPrompt || SYSTEM
    this.history = []; this.mcp = undefined; this.controller = undefined; this.sessionId = randomUUID()
  }
  async connect() {
    if (this.mcp) return
    const config = this.gate.config
    const env = Object.fromEntries(['PATH', 'SystemRoot', 'WINDIR', 'HOME', 'TMPDIR', 'TEMP']
      .filter(k => process.env[k]).map(k => [k, process.env[k]]))
    this.mcp = new RpcProcess(config.command, config.args, { env: { ...env, ...config.env } })
    await this.mcp.request('initialize', { protocolVersion: '2025-06-18', capabilities: {},
      clientInfo: { name: 'SimRep AI', version: '0.1.0' } })
    this.mcp.send({ jsonrpc: '2.0', method: 'notifications/initialized' })
    const list = await this.mcp.request('tools/list')
    this.tools = list.tools.filter(t => TOOL_NAMES.includes(t.name))
  }
  async call(name, args) {
    if (!this.tools.some(t => t.name === name) || !args || typeof args !== 'object' || Array.isArray(args)) {
      throw new BridgeError('TOOL_DENIED', 'Only registered SimRep MCP tools with object arguments can be called.')
    }
    if (this.controller.signal.aborted) throw new BridgeError('CANCELLED', 'Turn cancelled.')
    // All execution passes the same proxy and UI approval gate as the SDKs.
    return this.mcp.request('tools/call', { name, arguments: args }, 3_600_000)
  }
  parseArguments(raw) {
    try { return typeof raw === 'string' ? JSON.parse(raw) : raw } catch {
      throw new BridgeError('PROVIDER_RESPONSE', 'The provider returned invalid tool arguments.')
    }
  }
  async post(route, body, credential) {
    const p = this.provider
    const token = credential.apiKey || credential.access_token
    if (typeof token !== 'string' || !token) throw new BridgeError('AUTH_REQUIRED', 'Provider credentials are unavailable.')
    const base = credential.baseUrl || p.baseUrl
    const headers = { 'content-type': 'application/json', accept: 'application/json', ...credential.headers,
      ...(p.wireApi === 'anthropic-messages' ? { 'anthropic-version': '2023-06-01',
        ...(p.id === 'kimi-coding' && !credential.apiKey ? { authorization: `Bearer ${token}` } : { 'x-api-key': token }) }
        : { authorization: `Bearer ${token}` }),
    }
    if (p.id === 'kimi-coding' && !credential.apiKey) Object.assign(headers, await this.auth.identityHeaders())
    if (p.id.startsWith('openrouter')) headers['x-session-id'] = this.sessionId
    if (p.id === 'github-copilot') Object.assign(headers, { 'Openai-Intent': 'conversation-panel', 'X-Initiator': this.history.some(m => m.role === 'tool') ? 'agent' : 'user' })
    let response
    try {
      response = await this.fetcher(base.replace(/\/$/, '') + route, { method: 'POST', redirect: 'error',
        signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(180_000)]), headers, body: JSON.stringify(body) })
    } catch {
      if (this.controller.signal.aborted) throw new BridgeError('CANCELLED', 'Turn cancelled.')
      throw new BridgeError('PROVIDER_ERROR', 'Provider request failed. Check connectivity and model access.')
    }
    if (!response.ok) throw new BridgeError(response.status === 401 ? 'AUTH_EXPIRED' : 'PROVIDER_ERROR',
      response.status === 401 ? 'Provider credentials expired. Sign in again.' : 'Provider rejected the request. Check model access and usage limits.', { status: response.status })
    let data
    try { data = await response.json() } catch { throw new BridgeError('PROVIDER_RESPONSE', 'Provider returned an invalid response.') }
    if (!data || typeof data !== 'object') throw new BridgeError('PROVIDER_RESPONSE', 'Provider returned an invalid response.')
    return data
  }
  async completion(credential) {
    const tools = this.tools.map(t => ({ type: 'function', function: { name: t.name, description: t.description, parameters: t.inputSchema } }))
    const data = await this.post('/chat/completions', { model: this.model, messages: [{ role: 'system', content: this.systemPrompt }, ...this.history],
      tools, stream: false, ...(this.provider.id.startsWith('openrouter') && { session_id: this.sessionId }) }, credential)
    const message = data.choices?.[0]?.message
    if (!message || message.role !== 'assistant') throw new BridgeError('PROVIDER_RESPONSE', 'Provider returned no assistant message.')
    this.history.push(message)
    if (typeof message.content === 'string' && message.content) this.emit({ type: 'delta', text: message.content })
    const calls = message.tool_calls || []
    if (!Array.isArray(calls)) throw new BridgeError('PROVIDER_RESPONSE', 'Invalid provider tool calls.')
    for (const call of calls) {
      if (call.type !== 'function' || typeof call.id !== 'string') throw new BridgeError('PROVIDER_RESPONSE', 'Invalid provider tool call.')
      const result = await this.call(call.function?.name, this.parseArguments(call.function?.arguments))
      this.history.push({ role: 'tool', tool_call_id: call.id, content: JSON.stringify(result) })
    }
    if (!calls.length && data.choices[0].finish_reason === 'length') throw new BridgeError('PROVIDER_RESPONSE', 'Provider output was truncated.')
    return calls.length > 0
  }
  async responses(credential) {
    const tools = this.tools.map(t => ({ type: 'function', name: t.name, description: t.description, parameters: t.inputSchema, strict: false }))
    const data = await this.post('/responses', { model: this.model, instructions: this.systemPrompt, input: this.history, tools, stream: false, store: false }, credential)
    if (!Array.isArray(data.output) || data.status === 'failed' || data.status === 'incomplete') throw new BridgeError('PROVIDER_RESPONSE', 'Provider did not complete its response.')
    this.history.push(...data.output)
    let called = false
    for (const item of data.output) {
      if (item.type === 'message') {
        for (const part of item.content || []) if (part.type === 'output_text' && typeof part.text === 'string') this.emit({ type: 'delta', text: part.text })
      } else if (item.type === 'function_call') {
        if (typeof item.call_id !== 'string') throw new BridgeError('PROVIDER_RESPONSE', 'Invalid provider tool call.')
        const result = await this.call(item.name, this.parseArguments(item.arguments))
        this.history.push({ type: 'function_call_output', call_id: item.call_id, output: JSON.stringify(result) }); called = true
      }
    }
    return called
  }
  async messages(credential) {
    const data = await this.post('/v1/messages', { model: this.model, system: this.systemPrompt, messages: this.history,
      max_tokens: 8192, tools: this.tools.map(t => ({ name: t.name, description: t.description, input_schema: t.inputSchema })), stream: false }, credential)
    if (!Array.isArray(data.content)) throw new BridgeError('PROVIDER_RESPONSE', 'Provider returned no message content.')
    if (data.stop_reason === 'max_tokens') throw new BridgeError('PROVIDER_RESPONSE', 'Provider output was truncated.')
    this.history.push({ role: 'assistant', content: data.content })
    const results = []
    for (const item of data.content) {
      if (item.type === 'text' && typeof item.text === 'string') this.emit({ type: 'delta', text: item.text })
      if (item.type === 'tool_use') {
        const result = await this.call(item.name, item.input)
        results.push({ type: 'tool_result', tool_use_id: item.id, content: JSON.stringify(result), is_error: result.isError === true })
      }
    }
    if (results.length) this.history.push({ role: 'user', content: results })
    return results.length > 0
  }
  async native(credential) {
    const stream = await this.nativeStream(this.model, { systemPrompt: this.systemPrompt, messages: this.history,
      tools: this.tools.map(t => ({ name: t.name, description: t.description, parameters: t.inputSchema })) },
    { apiKey: credential.access_token, signal: this.controller.signal, cascadeId: this.sessionId, sessionId: this.sessionId })
    for await (const event of stream) if (event.type === 'text_delta') this.emit({ type: 'delta', text: event.delta })
    const message = await stream.result()
    if (['error', 'aborted', 'length'].includes(message.stopReason)) throw new BridgeError('PROVIDER_ERROR', 'Devin could not complete this turn. Check login, model access and limits.')
    this.history.push(message)
    let called = false
    for (const item of message.content) if (item.type === 'toolCall') {
      if (item.incomplete) throw new BridgeError('PROVIDER_RESPONSE', 'Provider tool call was truncated.')
      const result = await this.call(item.name, item.arguments)
      this.history.push({ role: 'toolResult', toolCallId: item.id, toolName: item.name,
        content: [{ type: 'text', text: JSON.stringify(result) }], isError: result.isError === true, timestamp: Date.now() })
      called = true
    }
    return called
  }
  async send(text) {
    this.controller = new AbortController()
    const before = this.history.length
    try {
      await this.connect()
      this.history.push({ role: 'user', content: text, ...(this.provider.wireApi === 'devin-agent' && { timestamp: Date.now() }) })
      for (let step = 0; step < 32; step++) {
        if (this.controller.signal.aborted) return 'interrupted'
        const credential = await this.auth.access(this.account)
        const again = this.provider.wireApi === 'chat-completions' ? await this.completion(credential)
          : this.provider.wireApi === 'responses' ? await this.responses(credential)
            : this.provider.wireApi === 'anthropic-messages' ? await this.messages(credential)
              : this.provider.wireApi === 'kiro' ? await kiroRound(this, credential) : await this.native(credential)
        if (!again) return this.controller.signal.aborted ? 'interrupted' : 'completed'
      }
      throw new BridgeError('PROVIDER_TOOL_LIMIT', 'This turn exceeded 32 tool-call rounds.')
    } catch (error) {
      // Never replay dangling tool calls after an interrupted or failed turn.
      this.history.splice(before)
      if (this.controller.signal.aborted) return 'interrupted'
      throw error
    }
  }
  cancel() { this.controller?.abort(); this.mcp?.close(); this.mcp = undefined }
  close() { this.cancel(); this.gate.close() }
}
