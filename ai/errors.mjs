export class BridgeError extends Error {
  constructor(code, message, details = undefined) {
    super(message)
    this.code = code
    this.details = details
  }
}

// Only errors produced at our own boundaries carry display text. SDK exceptions
// can contain request headers, URLs or credential material: never relay them.
export function publicError(error) {
  return error instanceof BridgeError
    ? { code: error.code, message: error.message, ...(error.details ? { details: error.details } : {}) }
    : { code: 'PROVIDER_ERROR', message: 'Provider operation failed. Check login, model access, and installation.' }
}

// Codex reports upstream HTTP status/body/code/request IDs in its turn error.
// Retain that diagnostic shape, but scrub credentials before crossing IPC.
export function providerDiagnostic(error, secrets = []) {
  const redact = text => {
    let safe = String(text)
    for (const secret of secrets) if (typeof secret === 'string' && secret.length > 0) safe = safe.split(secret).join('[redacted]')
    return safe.replace(/Bearer\s+[^\s"']+/gi, 'Bearer [redacted]')
      .replace(/https:\/\/auth\.openai\.com\/[^\s"']+/gi, '[authorization URL redacted]')
      .replace(/("(?:access_token|refresh_token|id_token|api_key)"\s*:\s*")[^"]*"/gi, '$1[redacted]"')
  }
  const message = redact(error?.message || 'ChatGPT could not complete this turn.').slice(0, 8000)
  const info = error?.codexErrorInfo
  return {
    code: 'PROVIDER_ERROR', message,
    details: {
      ...(info ? { upstream: JSON.parse(redact(JSON.stringify(info))) } : {}),
      ...(error?.additionalDetails ? { additionalDetails: redact(error.additionalDetails).slice(0, 8000) } : {}),
    },
  }
}
