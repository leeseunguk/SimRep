import { createHash } from 'node:crypto'
import { readFile, readdir, lstat } from 'node:fs/promises'
import path from 'node:path'

const fileKeys = new Set(['pa', 'cuts', 'particles', 'run_file', 'bfield_map', 'magnetic_vector_potential', 'magnetic_pa', 'bfield_coil', 'mbmr', 'path', 'potential', 'components', 'base', 'file'])
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

/** Content identity for the external files consumed by a workflow and their field families. */
export async function inputFingerprint(value, inputFile) {
  const visiting = new Set()
  async function file(relative, parent) {
    const absolute = inputFile(parent ? path.resolve(parent, relative) : relative, 'optimization dependency')
    if (visiting.has(absolute)) throw new Error(`cyclic optimization dependency: ${relative}`)
    visiting.add(absolute)
    try {
      const bytes = await readFile(absolute)
      const result = { sha256: sha256(bytes) }
      if (/\.pa0$/i.test(absolute)) {
        const dir = path.dirname(absolute), stem = path.basename(absolute).slice(0, -4)
        const names = (await readdir(dir)).filter(name => name.slice(0, stem.length) === stem && /^\.pa(?:[1-9][0-9]*|_)$/i.test(name.slice(stem.length))).sort()
        result.bases = {}
        for (const name of names) result.bases[name.slice(stem.length).toLowerCase()] = await file(path.join(dir, name))
      }
      if (/\.pa(?:[0-9]+|_|#)?$/i.test(absolute)) {
        for (const extension of ['cuts.json', 'mag.json']) {
          const sibling = absolute.replace(/\.[^.]+$/, `.${extension}`)
          let exists
          try { exists = (await lstat(sibling)).isFile() } catch (error) { if (error.code !== 'ENOENT') throw error }
          if (exists) result[extension] = await file(sibling)
        }
      }
      if (/\.json$/i.test(absolute)) {
        const parsed = JSON.parse(bytes)
        // Run settings use the simulator workspace; exported map components
        // resolve alongside their sidecar.
        result.dependencies = await walk(parsed, parsed.physics ? undefined : path.dirname(absolute))
      }
      return result
    } finally { visiting.delete(absolute) }
  }
  async function walk(node, parent) {
    if (Array.isArray(node)) {
      const result = []
      for (const item of node) result.push(await walk(item, parent))
      return result
    }
    if (!node || typeof node !== 'object') return node
    const result = {}
    for (const [key, item] of Object.entries(node)) {
      if (fileKeys.has(key) && typeof item === 'string') result[key] = await file(item, parent)
      else if (fileKeys.has(key) && Array.isArray(item)) {
        result[key] = []
        for (const value of item) result[key].push(typeof value === 'string' ? await file(value, parent) : await walk(value, parent))
      } else result[key] = await walk(item, parent)
    }
    return result
  }
  return walk(value)
}
