/** Checks are independent of provider prose and survive output compaction. */
export function verifyResult(value) {
  const issues = []
  function visit(node, location = '') {
    if (!node || typeof node !== 'object') return
    if (node.converged === false) issues.push(`${location}solver did not converge; inspect cycles, residual and electrode constraints`)
    if (node.fallback_reason) issues.push(`${location}fallback: ${node.fallback_reason}`)
    for (const key of ['device', 'precision', 'backend', 'surface_mode']) {
      const wanted = node.requested?.[key], got = node.executed?.[key]
      if (wanted != null && wanted !== 'auto' && got != null && wanted !== got) issues.push(`${location}requested ${key}=${wanted}, executed ${got}`)
    }
    if (node.gap_check) {
      const gap = node.gap_check
      if (gap.mode === 'off') issues.push(`${location}gap check was disabled`)
      if (gap.warnings?.length || gap.pairs?.some(pair => pair.status !== 'ok')) issues.push(`${location}gap check has unresolved or under-resolved gaps; refine spacing (target gap/10)`)
    }
    if (node.error) issues.push(`${location}tool failed: ${node.error.message ?? node.error}`)
    if (Object.hasOwn(node, 'u_num') && node.u_num === null) issues.push(`${location}numerical uncertainty is indeterminate; refine the grid study before claiming convergence`)
    for (const [key, child] of Object.entries(node)) {
      if (key === 'verification') continue
      if (child && typeof child === 'object') visit(child, `${location}${key}.`)
    }
  }
  visit(value)
  const unique = [...new Set(issues)]
  return { ok: !unique.length, issues: unique.slice(0, 8), issue_count: unique.length, timing_metric: 'compute_ms' }
}

export function parseToolResult(result) {
  const text = result?.content?.filter(item => item.type === 'text').map(item => item.text).join('\n') ?? ''
  try { return JSON.parse(text) } catch { return null }
}

/** Buffer claims until checked; only an exact compute_ms measurement may become a speed number. */
export function checkClaims(text, computeValues) {
  const notes = []
  const safe = text.split(/(?<=\n)|(?<=[.!?])\s+(?=[A-Z가-힣])/u).filter(sentence => {
    const unsupportedMetric = /\b(?:prep_ms|setup_ms|total_ms|elapsed_ms|end_to_end_ms|kernel_ms|submit_readback_ms)["'\s:=]+\d/i.test(sentence)
    const metricValues = [...sentence.matchAll(/\bcompute_ms["'\s:=]+(\d+(?:\.\d+)?)/g)]
    if (unsupportedMetric || metricValues.some(([, value]) => !computeValues.some(observed => Math.abs(observed - Number(value)) <= Math.max(1e-9, observed * 0.00001)))) {
      notes.push('Speed claim withheld: report only measured compute_ms, never prep_ms/setup_ms/total_ms or an unverified speedup.')
      return false
    }
    const speed = /(?:speed|faster|runtime|elapsed|wall.clock|compute|took|milliseconds|refin|solv|flight|속도|시간|걸렸|소요|배\s*빠)/i.test(sentence)
    const timings = [...sentence.matchAll(/(\d+(?:\.\d+)?)\s*(ms|milliseconds?|seconds?|s|초|밀리초)\b/gi)]
    const ratio = /\d+(?:\.\d+)?\s*(?:x|times|배)\b/i.test(sentence)
    if (!timings.length && !(speed && ratio)) return true
    const valid = !ratio && /compute_ms/.test(sentence) && timings.length > 0 && timings.every(([, number, unit]) =>
      computeValues.some(value => Math.abs(value - Number(number) * (/^(s|second|초)/i.test(unit) ? 1000 : 1)) <= Math.max(1e-9, value * 0.00001)))
    if (!valid) notes.push('Speed claim withheld: report only measured compute_ms, never prep_ms/setup_ms/total_ms or an unverified speedup.')
    return valid
  }).join('')
  return { text: safe, notes: [...new Set(notes)] }
}
