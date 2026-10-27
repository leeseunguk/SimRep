import { readdir, readFile, realpath, stat } from 'node:fs/promises'
import path from 'node:path'

const INTERESTING = /\.(gem|step|stp|iges|igs|pa#|pa0|cuts\.json|ion|fly2|csv|json)$/i
const SKIP = new Set(['.git', 'node_modules', 'target', '.venv', '.codex', '.claude', '.simrep-ai'])

/** Bounded inventory; never follow directory symlinks or read executable workspace instructions. */
export async function inspectWorkspace(root) {
  root = await realpath(root)
  const files = []
  let visited = 0, truncated = false
  async function visit(directory, depth) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (++visited > 2000 || files.length >= 150) { truncated = true; return }
      const absolute = path.join(directory, entry.name)
      if (entry.isDirectory() && depth < 3 && !SKIP.has(entry.name)) await visit(absolute, depth + 1)
      else if (entry.isFile() && INTERESTING.test(entry.name)) {
        const info = await stat(absolute)
        files.push({ path: path.relative(root, absolute), bytes: info.size, modified: info.mtimeMs })
      }
    }
  }
  await visit(root, 0)
  const results = []
  for (const file of files.filter(file => /\.json$/i.test(file.path) && !file.path.endsWith('.cuts.json') && file.bytes <= 64_000)
    .sort((a, b) => b.modified - a.modified).slice(0, 5)) {
    let value
    try { value = JSON.parse(await readFile(path.join(root, file.path), 'utf8')) } catch { continue }
    if (!value || typeof value !== 'object' || !('schema_version' in value)) continue
    results.push({ path: file.path, ...Object.fromEntries(['converged', 'cycles', 'executed', 'requested', 'fallback_reason', 'compute_ms', 'arrivals', 'final_residual']
      .filter(key => value[key] !== undefined).map(key => [key, value[key]])) })
  }
  return { files: files.map(({ path, bytes }) => ({ path, bytes })), last_results: results, truncated }
}
