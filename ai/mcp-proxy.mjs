// The only MCP server exposed to either agent. Every mutating call passes a
// user-approval gate before the workspace-restricted real server can see it.
import readline from 'node:readline'
import path from 'node:path'
import { randomUUID } from 'node:crypto'
import { RpcProcess } from './rpc.mjs'
import { HERE, TOOL_NAMES } from './tools.mjs'

const env = Object.fromEntries(['PATH', 'SystemRoot', 'WINDIR', 'HOME', 'TMPDIR', 'TEMP', 'SIMREP_BIN', 'SIMREP_WORKSPACE', 'SIMREP_PYTHON']
  .filter(k => process.env[k]).map(k => [k, process.env[k]]))
const mcp = new RpcProcess(process.execPath, [path.join(HERE, '..', 'mcp', 'server.mjs')], { env })
const send = message => process.stdout.write(JSON.stringify(message) + '\n')
const gateUrl = new URL(process.env.SIMREP_AI_GATE)
if (gateUrl.protocol !== 'http:' || gateUrl.hostname !== '127.0.0.1' || gateUrl.pathname !== '/tool') throw new Error('Invalid gate')

async function gate(message) {
  const response = await fetch(gateUrl, {
    method: 'POST', redirect: 'error', signal: AbortSignal.timeout(125_000),
    headers: { authorization: `Bearer ${process.env.SIMREP_AI_GATE_KEY}`, 'content-type': 'application/json' },
    body: JSON.stringify(message),
  })
  if (!response.ok) throw new Error('Tool gate unavailable')
  return response.json()
}

const NOTIFICATIONS = new Set(['notifications/initialized', 'notifications/cancelled', 'notifications/progress'])
const lines = readline.createInterface({ input: process.stdin })
lines.on('line', line => {
  void (async () => {
    let message
    try { message = JSON.parse(line) } catch { return }
    if (!message || typeof message !== 'object' || Array.isArray(message)) return
    if (message.id === undefined || message.id === null) {
      if (NOTIFICATIONS.has(message.method)) mcp.send(message)
      else console.error(JSON.stringify({ type: 'audit', event: 'rejected', method: message.method === 'tools/call' ? 'tools/call' : 'unknown', reason: 'request_id_required' }))
      return
    }
    try {
      let callId
      if (message.method === 'tools/call') {
        callId = randomUUID()
        if (!TOOL_NAMES.includes(message.params?.name)) throw new Error('Unknown tool')
        const decision = await gate({ type: 'call', callId, name: message.params.name, arguments: message.params.arguments || {} })
        if (!decision.allow) {
          const result = { isError: true, content: [{ type: 'text', text: 'User declined tool execution.' }] }
          await gate({ type: 'result', callId, name: message.params.name, result })
          send({ jsonrpc: '2.0', id: message.id, result })
          return
        }
      }
      const result = await mcp.request(message.method, message.params, message.method === 'tools/call' ? 3_600_000 : 60_000)
      if (callId) await gate({ type: 'result', callId, name: message.params.name, result })
      send({ jsonrpc: '2.0', id: message.id, result })
    } catch {
      send({ jsonrpc: '2.0', id: message.id, error: { code: -32000, message: 'SimRep tool failed or was denied.' } })
    }
  })()
})
const close = () => { mcp.close(); process.exit(0) }
lines.on('close', close)
process.on('SIGTERM', close)
process.on('SIGINT', close)
