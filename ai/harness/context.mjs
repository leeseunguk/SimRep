import { randomUUID } from 'node:crypto'
import { verifyResult } from './verification.mjs'

const IMPORTANT = new Set(['schema_version', 'error', 'requested', 'executed', 'converged', 'cycles', 'final_residual', 'tolerance',
  'compute_ms', 'fallback_reason', 'gap_check', 'arrivals', 'ion_count', 'splat_counts', 'output_paths', 'verification'])

function bounded(value, depth = 0) {
  if (typeof value === 'string') return value.slice(0, 500)
  if (!value || typeof value !== 'object') return value
  if (depth >= 3) return '[details in output handle]'
  if (Array.isArray(value)) return value.slice(0, 8).map(item => bounded(item, depth + 1))
  return Object.fromEntries(Object.entries(value).slice(0, 12).map(([key, item]) => [key, bounded(item, depth + 1)]))
}

export class OutputStore {
  constructor({ limit = 12_000, capacity = 32 } = {}) { this.limit = limit; this.capacity = capacity; this.outputs = new Map() }
  compact(text) {
    if (text.length <= this.limit) return text
    const handle = randomUUID()
    this.outputs.set(handle, text)
    if (this.outputs.size > this.capacity) this.outputs.delete(this.outputs.keys().next().value)
    let value
    try { value = JSON.parse(text) } catch { value = null }
    const summary = value && typeof value === 'object'
      ? bounded(Object.fromEntries(Object.entries(value).filter(([key]) => IMPORTANT.has(key)))) : {}
    // Even a very large gap report must not consume the entire model context.
    delete summary.gap_check
    const result = { ...summary, verification: bounded(verifyResult(value)), truncated: true, handle, characters: text.length,
      preview: text.slice(0, Math.min(1200, this.limit / 4)), fetch: 'fetch_tool_output({handle, offset, length})' }
    if (JSON.stringify(result).length > this.limit) delete result.preview
    for (const key of Object.keys(summary)) if (JSON.stringify(result).length > this.limit) delete result[key]
    if (JSON.stringify(result).length > this.limit) result.verification = { ok: result.verification.ok, issue_count: result.verification.issue_count }
    if (JSON.stringify(result).length > this.limit) delete result.fetch
    const serialized = JSON.stringify(result)
    if (serialized.length > this.limit) throw new Error('Output limit too small for a handle')
    return serialized
  }
  fetch({ handle, offset = 0, length = 6000 }) {
    if (!this.outputs.has(handle)) throw new Error('Unknown or expired output handle')
    if (!Number.isInteger(offset) || offset < 0 || !Number.isInteger(length) || length < 1 || length > 12000) throw new Error('offset >= 0 and length 1..12000 must be integers')
    const text = this.outputs.get(handle)
    return { handle, offset, text: text.slice(offset, offset + length), next_offset: offset + length < text.length ? offset + length : null, characters: text.length }
  }
}
