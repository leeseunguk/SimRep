#!/usr/bin/env node
import readline from 'node:readline'
import { Bridge } from './bridge.mjs'

if (Number(process.versions.node.split('.')[0]) < 18) {
  process.stderr.write('SimRep AI requires Node.js 18.18 or newer.\n')
  process.exit(1)
}
const send = message => process.stdout.write(JSON.stringify(message) + '\n')
const bridge = new Bridge({ notify: send })
const lines = readline.createInterface({ input: process.stdin })
lines.on('line', line => {
  if (line.length > 2_000_000) { send({ jsonrpc: '2.0', id: null, error: { code: -32600, message: 'Request too large.' } }); return }
  let request
  try { request = JSON.parse(line) } catch {
    send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error.' } }); return
  }
  void bridge.handle(request).then(send)
})
function close() { bridge.close(); process.exit(0) }
lines.on('close', close)
process.on('SIGTERM', close)
process.on('SIGINT', close)
