import { createServer } from 'node:http'
import { randomBytes } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { BridgeError } from './errors.mjs'

export const READ_ONLY = ['gpu_info', 'pa_info', 'read_csv_summary', 'cad_measure']
export const TOOL_NAMES = [...READ_ONLY, 'gem2pa', 'mesh_import', 'refine', 'refine_geometry', 'fly',
  'export_geometry', 'export_grid', 'export_isosurface', 'export_trajectories']
export const HERE = path.dirname(fileURLToPath(import.meta.url))

// An authenticated private loopback channel connects provider-owned MCP proxies
// to our UI approvals. No provider token is part of this channel.
export async function toolGate(workspace, approve, emit) {
  const key = randomBytes(32).toString('hex')
  const server = createServer((req, res) => {
    if (req.method !== 'POST' || req.url !== '/tool' || req.headers.authorization !== `Bearer ${key}` || req.headers.origin) {
      res.writeHead(403); res.end(); return
    }
    let body = ''
    req.on('data', chunk => { body += chunk; if (body.length > 8_000_000) req.destroy() })
    req.on('end', () => {
      void (async () => {
        const message = JSON.parse(body)
        if (!TOOL_NAMES.includes(message.name)) throw new BridgeError('TOOL_DENIED', 'Unknown SimRep tool.')
        if (message.type === 'call') {
          emit({ type: 'tool_call', callId: message.callId, name: message.name, arguments: message.arguments })
          const allow = READ_ONLY.includes(message.name) || await approve(message.name, message.arguments)
          res.setHeader('content-type', 'application/json')
          res.end(JSON.stringify({ allow }))
        } else if (message.type === 'result') {
          emit({ type: 'tool_result', callId: message.callId, name: message.name, result: message.result })
          res.end('{}')
        } else { res.writeHead(400); res.end() }
      })().catch(() => { if (!res.headersSent) res.writeHead(400); res.end() })
    })
  })
  await new Promise((yes, no) => { server.once('error', no); server.listen(0, '127.0.0.1', () => yes(undefined)) })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('No tool gate port')
  return {
    config: {
      command: process.execPath,
      args: [path.join(HERE, 'mcp-proxy.mjs')],
      env: {
        SIMREP_WORKSPACE: workspace,
        SIMREP_BIN: process.env.SIMREP_BIN || '',
        SIMREP_AI_GATE: `http://127.0.0.1:${address.port}/tool`,
        SIMREP_AI_GATE_KEY: key,
      },
    },
    close: () => { server.closeAllConnections(); server.close() },
  }
}
