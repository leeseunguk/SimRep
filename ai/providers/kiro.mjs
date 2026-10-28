/*
 * Adapted from eddieparc/omo-accounts at
 * 3df128a51458c829e61a4da62d6545ed55bcc1c6:
 * src/providers/kiro/{config,oauth,vendor/kiro,vendor/eventstream}.ts.
 * modified for SimRep: MCP-only tools, strict frame errors, no debug logging.
 * MIT License
 * Copyright (c) 2026 eddieparc
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
import { randomUUID } from 'node:crypto'
import { BridgeError } from '../errors.mjs'

const crcTable = Uint32Array.from({ length: 256 }, (_, i) => {
  let c = i
  for (let bit = 0; bit < 8; bit++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  return c >>> 0
})
export function crc32(bytes) {
  let c = 0xffffffff
  for (const byte of bytes) c = crcTable[(c ^ byte) & 255] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}
function invalidFrame() { return new BridgeError('PROVIDER_RESPONSE', 'Kiro returned an invalid event stream.') }
export function parseFrame(frame) {
  const length = frame.readUInt32BE(0), headerLength = frame.readUInt32BE(4)
  if (length !== frame.length || headerLength > length - 16 ||
    frame.readUInt32BE(8) !== crc32(frame.subarray(0, 8)) ||
    frame.readUInt32BE(length - 4) !== crc32(frame.subarray(0, length - 4))) throw invalidFrame()
  const headers = {}
  let cursor = 12
  const end = 12 + headerLength
  while (cursor < end) {
    const nameLength = frame[cursor++]
    if (cursor + nameLength + 3 > end) throw invalidFrame()
    const name = frame.subarray(cursor, cursor + nameLength).toString(); cursor += nameLength
    if (frame[cursor++] !== 7) throw invalidFrame()
    const valueLength = frame.readUInt16BE(cursor); cursor += 2
    if (cursor + valueLength > end) throw invalidFrame()
    headers[name] = frame.subarray(cursor, cursor + valueLength).toString(); cursor += valueLength
  }
  let payload
  try { payload = JSON.parse(frame.subarray(end, length - 4).toString()) } catch { throw invalidFrame() }
  return { headers, payload }
}
export async function* kiroEvents(body) {
  let buffer = Buffer.alloc(0)
  for await (const chunk of body) {
    buffer = Buffer.concat([buffer, chunk])
    while (buffer.length >= 4) {
      const length = buffer.readUInt32BE(0)
      if (length < 16 || length > 16_000_000) throw invalidFrame()
      if (buffer.length < length) break
      yield parseFrame(buffer.subarray(0, length))
      buffer = buffer.subarray(length)
    }
    if (buffer.length > 16_000_000) throw invalidFrame()
  }
  if (buffer.length) throw invalidFrame()
}
export async function kiroRound(session, credential) {
  const history = session.history.map(m => m.role === 'user'
    ? { userInputMessage: { content: m.content, modelId: session.model } } : structuredClone(m))
  const current = history.pop()
  if (!current?.userInputMessage) throw new BridgeError('PROVIDER_RESPONSE', 'Invalid Kiro conversation state.')
  const tools = session.tools.map(t => ({ toolSpecification: { name: t.name, description: t.description,
    inputSchema: { json: t.inputSchema } } }))
  Object.assign(current.userInputMessage, { origin: 'AI_EDITOR', modelId: session.model,
    userInputMessageContext: { ...current.userInputMessage.userInputMessageContext, tools } })
  history.unshift({ userInputMessage: { content: `<SimRep system instructions>\n${session.systemPrompt}\n</SimRep system instructions>`, modelId: session.model } },
    { assistantResponseMessage: { content: 'I will follow the SimRep system instructions.' } })
  const body = { conversationState: { chatTriggerType: 'MANUAL', conversationId: session.sessionId,
    currentMessage: current, history }, inferenceConfig: { maxTokens: 32_000 },
    ...(credential.profileArn && { profileArn: credential.profileArn }) }
  let response
  try {
    response = await session.fetcher(session.provider.baseUrl, { method: 'POST', redirect: 'error',
      signal: AbortSignal.any([session.controller.signal, AbortSignal.timeout(180_000)]),
      headers: { authorization: `Bearer ${credential.access_token}`, 'content-type': 'application/json',
        accept: 'application/vnd.amazon.eventstream', 'X-Amz-Target': 'AmazonCodeWhispererStreamingService.GenerateAssistantResponse',
        'User-Agent': 'AWS-SDK-JS/3.0.0 kiro-ide/1.0.0', 'X-Amz-User-Agent': 'aws-sdk-js/3.0.0 kiro-ide/1.0.0',
        'Amz-Sdk-Request': 'attempt=1; max=1', 'Amz-Sdk-Invocation-Id': randomUUID(),
        'x-amzn-bedrock-cache-control': 'enable', 'anthropic-beta': 'prompt-caching-2024-07-31' },
      body: JSON.stringify(body) })
  } catch {
    throw new BridgeError('PROVIDER_ERROR', 'Kiro request failed. Check connectivity and sign-in.')
  }
  if (!response.ok) throw new BridgeError([401, 403].includes(response.status) ? 'AUTH_EXPIRED' : 'PROVIDER_ERROR',
    'Kiro rejected the request. Check sign-in, model access and usage limits.', { status: response.status })
  if (!response.body) throw invalidFrame()
  let content = '', sawEvent = false
  const calls = new Map()
  for await (const { headers, payload } of kiroEvents(response.body)) {
    sawEvent = true
    if (headers[':message-type'] === 'exception' || headers[':message-type'] === 'error') throw new BridgeError('PROVIDER_ERROR', 'Kiro reported a provider error.')
    const type = headers[':event-type']
    if (type === 'assistantResponseEvent' || type === 'codeEvent') {
      const text = payload.content ?? payload.text
      if (typeof text === 'string') { content += text; session.emit({ type: 'delta', text }) }
    } else if (type === 'toolUseEvent') {
      for (const item of Array.isArray(payload) ? payload : [payload]) {
        if (typeof item.toolUseId !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(item.toolUseId)) throw invalidFrame()
        const call = calls.get(item.toolUseId) || { id: item.toolUseId, name: item.name, input: '', ended: false }
        if (typeof item.name === 'string') call.name = item.name
        if (item.input !== undefined) call.input += typeof item.input === 'string' ? item.input : JSON.stringify(item.input)
        if (item.stop === true) call.ended = true
        calls.set(item.toolUseId, call)
      }
    }
  }
  if (!sawEvent) throw invalidFrame()
  const uses = [...calls.values()].map(call => {
    if (!call.ended) throw new BridgeError('PROVIDER_RESPONSE', 'Kiro tool call ended before its completion marker.')
    return { toolUseId: call.id, name: call.name, input: session.parseArguments(call.input || '{}') }
  })
  session.history.push({ assistantResponseMessage: { content: uses.length ? '' : content,
    ...(uses.length && { messageId: randomUUID(), toolUses: uses }) } })
  if (uses.length) {
    const results = []
    for (const use of uses) {
      const result = await session.call(use.name, use.input)
      results.push({ toolUseId: use.toolUseId, status: result.isError ? 'error' : 'success', content: [{ text: JSON.stringify(result) }] })
    }
    session.history.push({ userInputMessage: { content: '', modelId: session.model, userInputMessageContext: { toolResults: results } } })
  }
  return uses.length > 0
}
