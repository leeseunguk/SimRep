/** Checks are independent of provider prose and survive output compaction. */
export function verifyResult(value) {
  const issues = []
  let fwhmUncertaintyAvailable = false
  function visit(node, location = '') {
    if (!node || typeof node !== 'object') return
    if (node.identical === false) issues.push(`${location}comparison reports differences`)
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
    if (Array.isArray(node.warnings) && node.warnings.includes('voltage_convention not stated; k may differ by a factor of 2 from literature conventions')) issues.push(`${location}voltage_convention not stated; k may differ by a factor of 2 from literature conventions`)
    if (Array.isArray(node.warnings) && Object.hasOwn(node, 'fwhm_v')) issues.push(...node.warnings.map(warning => `${location}${warning}`))
    if (node.bootstrap?.fwhm_v?.sd != null) fwhmUncertaintyAvailable = true
    if (node.error) issues.push(`${location}tool failed: ${node.error.message ?? node.error}`)
    if (Object.hasOwn(node, 'u_num') && node.u_num === null) issues.push(`${location}numerical uncertainty is indeterminate; refine the grid study before claiming convergence`)
    for (const [key, child] of Object.entries(node)) {
      if (key === 'verification') continue
      if (child && typeof child === 'object') visit(child, `${location}${key}.`)
    }
  }
  visit(value)
  const unique = [...new Set(issues)]
  return { ok: !unique.length, issues: unique.slice(0, 8), issue_count: unique.length, timing_metric: 'compute_ms', fwhm_uncertainty_available: fwhmUncertaintyAvailable }
}

export function parseToolResult(result) {
  const text = result?.content?.filter(item => item.type === 'text').map(item => item.text).join('\n') ?? ''
  try { return JSON.parse(text) } catch { return null }
}

/** Preserve prose; annotate unsupported computational speed claims. */
export function checkClaims(text, computeValues, fwhmUncertaintyAvailable = false) {
  const claims = text.split(/(?<=\n)|(?<=[.!?])\s+(?=[A-Z가-힣])/u)
  const unsupported = claims.some(sentence => {
    if (/\b(?:time of flight|tof)\b/i.test(sentence) && !/\b(?:faster|speedup|speed|runtime|elapsed)\b/i.test(sentence)) return false
    const ratio = /\d+(?:\.\d+)?\s*(?:x|times|배)\b/i.test(sentence) && /(?:faster|slower|speed|빠|느)/i.test(sentence)
    const timing = /(?:speed|runtime|elapsed|wall.clock|compute|took|solver|refin|속도|걸렸|소요)/i.test(sentence) && /\d+(?:\.\d+)?\s*(?:ms|milliseconds?|seconds?|초|밀리초)\b/i.test(sentence)
    const metrics = [...sentence.matchAll(/\bcompute_ms["'\s:=]+(\d+(?:\.\d+)?)/gi)]
    const noncompute = /\b(?:prep_ms|setup_ms|total_ms|elapsed_ms|end_to_end_ms|kernel_ms|submit_readback_ms)\b/i.test(sentence)
    const backed = metrics.length > 0 && metrics.every(([, value]) => computeValues.some(observed => Math.abs(observed - Number(value)) <= Math.max(1e-9, Math.abs(observed) * 0.00001)))
    return noncompute || ((ratio || timing || metrics.length > 0) && !backed) || (ratio && metrics.length < 2)
  })
  const notes = unsupported ? ['Speed or ratio claim is not verified by session compute_ms results.'] : []
  if (fwhmUncertaintyAvailable && claims.some(sentence => /\bFWHM\b/i.test(sentence) && /\d/.test(sentence) && !/(?:±|\+\s*\/\s*-|uncertainty|standard deviation|\bsd\b|confidence interval|\bCI\b|불확도|표준편차|신뢰구간)/i.test(sentence))) notes.push('FWHM was quoted without its available bootstrap uncertainty.')
  return { text, notes }
}
