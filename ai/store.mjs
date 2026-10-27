import { createHash, randomUUID } from 'node:crypto'
import { mkdir, lstat, chmod, open, readFile, rename, unlink, readdir, realpath } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { BridgeError } from './errors.mjs'

export function configDirectory() {
  if (process.env.SIMREP_AI_CONFIG_DIR) return path.resolve(process.env.SIMREP_AI_CONFIG_DIR)
  const base = process.platform === 'win32' ? process.env.APPDATA
    : process.platform === 'darwin' ? path.join(os.homedir(), 'Library', 'Application Support')
      : process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config')
  return path.join(base || os.homedir(), 'dev.simrep.app', 'ai')
}

export function accountId(client, subject) {
  return createHash('sha256').update(JSON.stringify([client, subject])).digest('hex')
}

export class Store {
  constructor(directory = configDirectory()) {
    this.directory = directory
    this.queue = Promise.resolve()
  }

  async init() {
    await mkdir(this.directory, { recursive: true, mode: 0o700 })
    const stat = await lstat(this.directory)
    if (!stat.isDirectory() || stat.isSymbolicLink()) throw new BridgeError('CONFIG_UNSAFE', 'Credential directory must be a real directory.')
    const real = await realpath(this.directory)
    const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
    if (real === repo || real.startsWith(repo + path.sep)) throw new BridgeError('CONFIG_UNSAFE', 'Credentials must be stored outside the repository.')
    await chmod(this.directory, 0o700)
  }

  async read(name) {
    try {
      const file = path.join(this.directory, name + '.json')
      const stat = await lstat(file)
      if (!stat.isFile() || stat.isSymbolicLink() || (process.platform !== 'win32' && (stat.mode & 0o077))) {
        throw new BridgeError('CONFIG_UNSAFE', 'Credential file permissions must be owner-only.')
      }
      return JSON.parse(await readFile(file, 'utf8'))
    } catch (error) {
      if (error.code === 'ENOENT') return undefined
      throw error
    }
  }

  async write(name, value) {
    const target = path.join(this.directory, name + '.json')
    const temp = target + '.' + randomUUID() + '.tmp'
    const handle = await open(temp, 'wx', 0o600)
    try {
      await handle.writeFile(JSON.stringify(value), 'utf8')
      await handle.sync()
    } finally {
      await handle.close()
    }
    try {
      await rename(temp, target)
    } catch (error) {
      await unlink(temp).catch(() => {})
      throw error
    }
  }

  async remove(name) {
    await unlink(path.join(this.directory, name + '.json')).catch(error => {
      if (error.code !== 'ENOENT') throw error
    })
  }

  // All mutations, including refresh, hold a cross-process lock. We fail busy
  // rather than race a rotating refresh token. Never auto-steal a stale lock:
  // two reclaimers could otherwise remove each other's newly acquired lock.
  locked(fn) {
    const task = this.queue.then(async () => {
      await this.init()
      const lock = path.join(this.directory, 'session.lock')
      let handle
      try {
        handle = await open(lock, 'wx', 0o600)
      } catch (error) {
        if (error.code !== 'EEXIST') throw error
        throw new BridgeError('BUSY', 'Credential store is locked. Close other SimRep AI processes; after a crash remove session.lock only when its owner has exited.')
      }
      await handle.writeFile(String(process.pid))
      try { return await fn() } finally { await handle.close(); await unlink(lock) }
    })
    this.queue = task.then(() => undefined, () => undefined)
    return task
  }

  async host() {
    return this.locked(async () => {
      let host = await this.read('host')
      if (!host) { host = { id: 'urn:uuid:' + randomUUID() }; await this.write('host', host) }
      return host.id
    })
  }

  async accounts() {
    await this.init()
    const names = await readdir(this.directory)
    return Promise.all(names.filter(n => /^registration-[a-f0-9]{64}\.json$/.test(n))
      .map(n => this.read(n.slice(0, -5))))
  }

  credential(id) {
    if (!/^[a-f0-9]{64}$/.test(id)) throw new BridgeError('INVALID_PARAMS', 'Unknown account.')
    return this.read('account-' + id)
  }
}
