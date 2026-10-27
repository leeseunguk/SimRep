import { spawn } from 'node:child_process'
import readline from 'node:readline'
import { BridgeError } from './errors.mjs'

export class RpcProcess {
  /** @param {string} command
   * @param {string[]} args
   * @param {import('node:child_process').SpawnOptionsWithoutStdio & {onStderr?: (data: string) => void}} options */
  constructor(command, args, options = {}) {
    const { onStderr, ...spawnOptions } = options
    this.child = spawn(command, args, { ...spawnOptions, shell: false, windowsHide: true, stdio: ['pipe', 'pipe', onStderr ? 'pipe' : 'ignore'] })
    if (onStderr) this.child.stderr.on('data', chunk => onStderr(String(chunk)))
    this.sequence = 0
    this.pending = new Map()
    this.onmessage = async (_message) => {}
    this.onclose = () => {}
    this.closed = false
    this.child.once('error', () => this.fail())
    this.child.once('close', () => this.fail())
    this.child.stdin.on('error', () => this.fail())
    this.lines = readline.createInterface({ input: this.child.stdout })
    this.lines.on('line', line => {
      let message
      try { message = JSON.parse(line) } catch { return }
      if (message.id !== undefined && !message.method) {
        const waiter = this.pending.get(message.id)
        if (!waiter) return
        this.pending.delete(message.id)
        clearTimeout(waiter.timer)
        if (message.error) waiter.reject(new BridgeError('PROVIDER_RPC', 'Provider rejected the operation. Check login and model access.', { code: message.error.code }))
        else waiter.resolve(message.result)
      } else {
        void this.onmessage(message).catch(() => this.close())
      }
    })
  }

  send(message) {
    if (this.closed) throw new BridgeError('PROVIDER_CLOSED', 'Provider process disconnected.')
    this.child.stdin.write(JSON.stringify(message) + '\n')
  }

  request(method, params = {}, timeoutMs = 60_000) {
    const id = ++this.sequence
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new BridgeError('PROVIDER_TIMEOUT', 'Provider request timed out.'))
      }, timeoutMs)
      this.pending.set(id, { resolve, reject, timer })
      try { this.send({ jsonrpc: '2.0', id, method, params }) } catch (error) {
        clearTimeout(timer); this.pending.delete(id); reject(error)
      }
    })
  }

  fail() {
    if (this.closed) return
    this.closed = true
    for (const { reject, timer } of this.pending.values()) {
      clearTimeout(timer)
      reject(new BridgeError('PROVIDER_CLOSED', 'Provider process disconnected.'))
    }
    this.pending.clear()
    this.onclose()
  }

  close() { this.fail(); this.lines.close(); this.child.kill() }
}
