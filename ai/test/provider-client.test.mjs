import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { ProviderSession } from '../providers/client.mjs'
import { PROVIDERS, providerById } from '../providers/table.mjs'
import { toolGate } from '../tools.mjs'
import { crc32, kiroEvents } from '../providers/kiro.mjs'

async function fixture(t, id, { tool = 'read_csv_summary', args = { path: 'data.csv' }, approve = async () => false, nativeStream = undefined } = {}) {
  const workspace = await mkdtemp(path.join(os.tmpdir(), 'simrep-provider-client-'))
  await writeFile(path.join(workspace, 'data.csv'), 'time_us\n2\n4\n')
  const events = [], requests = [], results = []
  const p = providerById(id)
  const server = createServer((req, res) => {
    void (async () => {
      let raw = ''; for await (const data of req) raw += data
      const body = JSON.parse(raw)
      requests.push({ body, headers: req.headers, url: req.url })
      const first = requests.length % 2 === 1
      if (p.wireApi === 'kiro') {
        assert.ok(JSON.stringify(body.conversationState.history).includes('Harness instructions'))
        assert.equal(body.conversationState.currentMessage.userInputMessage.userInputMessageContext.tools[0].toolSpecification.name, 'gpu_info')
        if (!first) results.push(JSON.parse(body.conversationState.currentMessage.userInputMessage.userInputMessageContext.toolResults[0].content[0].text))
        res.writeHead(200, { 'content-type': 'application/vnd.amazon.eventstream' })
        const bytes = first ? Buffer.concat([
          frame('toolUseEvent', { toolUseId: 'call_one', name: tool, input: JSON.stringify(args).slice(0, 4) }),
          frame('toolUseEvent', { toolUseId: 'call_one', input: JSON.stringify(args).slice(4), stop: true }),
        ]) : frame('assistantResponseEvent', { content: 'Completed.' })
        res.write(bytes.subarray(0, 9)); res.end(bytes.subarray(9)); return
      }
      assert.ok(req.headers.authorization === 'Bearer private-test-key' || req.headers['x-api-key'] === 'private-test-key')
      assert.equal(JSON.stringify(body).includes('private-test-key'), false)
      const definitions = body.tools.map(t => t.function?.name || t.name)
      assert.ok(definitions.includes('read_csv_summary')); assert.equal(definitions.includes('Bash'), false)
      res.setHeader('content-type', 'application/json')
      let reply
      if (p.wireApi === 'responses') {
        assert.equal(body.instructions, 'Harness instructions')
        if (!first) results.push(JSON.parse(body.input.findLast(m => m.type === 'function_call_output').output))
        reply = { status: 'completed', output: first ? [{ type: 'function_call', call_id: 'call-one', name: tool, arguments: JSON.stringify(args) }]
          : [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Completed.' }] }] }
      } else if (p.wireApi === 'anthropic-messages') {
        assert.equal(body.system, 'Harness instructions')
        if (!first) results.push(JSON.parse(body.messages.at(-1).content[0].content))
        reply = { stop_reason: first ? 'tool_use' : 'end_turn', content: first ? [{ type: 'tool_use', id: 'call-one', name: tool, input: args }]
          : [{ type: 'text', text: 'Completed.' }] }
      } else {
        assert.equal(body.messages[0].content, 'Harness instructions')
        if (!first) results.push(JSON.parse(body.messages.at(-1).content))
        reply = { choices: [{ finish_reason: first ? 'tool_calls' : 'stop', message: { role: 'assistant', content: first ? null : 'Completed.',
          ...(first && { tool_calls: [{ type: 'function', id: 'call-one', function: { name: tool, arguments: JSON.stringify(args) } }] }) } }] }
      }
      res.end(JSON.stringify(reply))
    })().catch(error => { results.push(error); res.writeHead(500); res.end('{}') })
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const origin = `http://127.0.0.1:${server.address().port}`
  const gate = await toolGate(workspace, approve, e => events.push(e))
  const auth = { access: async () => ({ apiKey: 'private-test-key', access_token: 'kiro-test-key', profileArn: 'arn:private-profile', baseUrl: origin }) }
  const session = new ProviderSession({ provider: { ...p, baseUrl: origin }, auth, workspace, model: p.defaultModel, gate,
    emit: e => events.push(e), systemPrompt: 'Harness instructions', ...(nativeStream && { nativeStream }) })
  t.after(async () => { session.close(); server.closeAllConnections(); server.close(); await rm(workspace, { recursive: true, force: true }) })
  return { session, events, requests, results, origin }
}

for (const p of PROVIDERS.filter(p => p.toolCalling && ['chat-completions', 'responses', 'anthropic-messages'].includes(p.wireApi))) {
  test(`${p.id}: ${p.authKind} ${p.wireApi} completes a real approved-proxy MCP call against a local fake provider`, async t => {
    const { session, events, requests, results } = await fixture(t, p.id)
    assert.equal(await session.send('Read data.csv'), 'completed')
    assert.equal(requests.length, 2)
    assert.equal(results[0].isError, false)
    const csv = JSON.parse(results[0].content[0].text)
    assert.equal(csv.row_count, 2); assert.equal(csv.numeric.time_us.mean, 3)
    assert.equal(events.filter(e => e.type === 'tool_call').length, 1)
    assert.equal(events.find(e => e.type === 'tool_result').result.isError, false)
    assert.equal(events.find(e => e.type === 'delta').text, 'Completed.')
    assert.equal(JSON.stringify(events).includes('private-test-key'), false)
    assert.equal(await session.send('Read it again'), 'completed')
    assert.equal(requests.length, 4)
  })
}
test('mutations use the existing approval gate and a denied tool never reaches the MCP server', async t => {
  let approvals = 0
  const { session, results } = await fixture(t, 'deepseek', { tool: 'gem2pa', args: { input: 'never-created.gem' },
    approve: async () => { approvals++; return false } })
  assert.equal(await session.send('Create a PA'), 'completed')
  assert.equal(approvals, 1); assert.equal(results[0].isError, true)
  assert.match(results[0].content[0].text, /declined/)
})
test('the same provider cannot read a path outside its selected workspace', async t => {
  const { session, results } = await fixture(t, 'minimax', { args: { path: '../outside.csv' } })
  await session.send('Read outside')
  assert.equal(results[0].isError, true)
})
test('unregistered tools never execute and provider errors do not expose response text', async t => {
  const { session, events } = await fixture(t, 'openai', { tool: 'Bash', args: { command: 'secret' } })
  await assert.rejects(session.send('Run a shell'), e => e.code === 'TOOL_DENIED')
  assert.equal(events.some(e => e.type === 'tool_call'), false)
  session.fetcher = async () => Response.json({ error: 'private-test-key' }, { status: 401 })
  await assert.rejects(session.send('Hello'), e => e.code === 'AUTH_EXPIRED' && !JSON.stringify(e).includes('private-test-key'))
})
test('Devin native adapter returns tool calls through the same MCP approval proxy', async t => {
  let rounds = 0
  const nativeStream = async (_model, context, options) => {
    assert.equal(context.systemPrompt, 'Harness instructions')
    assert.equal(options.apiKey, 'kiro-test-key')
    const first = ++rounds === 1
    if (!first) {
      const result = context.messages.at(-1)
      assert.equal(result.role, 'toolResult')
      assert.equal(JSON.parse(JSON.parse(result.content[0].text).content[0].text).numeric.time_us.mean, 3)
    }
    const message = { role: 'assistant', content: first ? [{ type: 'toolCall', id: 'native-one', name: 'read_csv_summary', arguments: { path: 'data.csv' } }]
      : [{ type: 'text', text: 'Native complete.' }], stopReason: first ? 'toolUse' : 'stop', timestamp: Date.now() }
    const stream = (async function* () { if (!first) yield { type: 'text_delta', delta: 'Native complete.' } })()
    stream.result = async () => message
    return stream
  }
  const { session, events } = await fixture(t, 'devin', { nativeStream })
  assert.equal(await session.send('Read data.csv'), 'completed')
  assert.equal(rounds, 2)
  assert.equal(events.find(e => e.type === 'tool_result').name, 'read_csv_summary')
})

function frame(type, payload) {
  const name = Buffer.from(':event-type'), value = Buffer.from(type)
  const header = Buffer.alloc(1 + name.length + 1 + 2 + value.length)
  header[0] = name.length; name.copy(header, 1); header[1 + name.length] = 7
  header.writeUInt16BE(value.length, 2 + name.length); value.copy(header, 4 + name.length)
  const data = Buffer.from(JSON.stringify(payload)), all = Buffer.alloc(16 + header.length + data.length)
  all.writeUInt32BE(all.length, 0); all.writeUInt32BE(header.length, 4); all.writeUInt32BE(crc32(all.subarray(0, 8)), 8)
  header.copy(all, 12); data.copy(all, 12 + header.length); all.writeUInt32BE(crc32(all.subarray(0, -4)), all.length - 4)
  return all
}
test('Kiro CodeWhisperer AWS eventstream drives a real MCP tool call over a fake HTTP server', async t => {
  const { session, requests, results, events } = await fixture(t, 'kiro')
  assert.equal(await session.send('Read data.csv'), 'completed')
  assert.equal(requests.length, 2)
  assert.equal(JSON.parse(results[0].content[0].text).numeric.time_us.mean, 3)
  assert.equal(events.find(e => e.type === 'delta').text, 'Completed.')
  assert.equal(requests[0].headers.authorization, 'Bearer kiro-test-key')
  assert.equal(requests[0].body.profileArn, 'arn:private-profile')
})
test('Kiro CRC failures and truncated event frames fail closed', async () => {
  assert.equal(crc32(Buffer.from('123456789')), 0xcbf43926)
  const corrupted = frame('toolUseEvent', { toolUseId: 'one', name: 'gem2pa', input: '{}', stop: true })
  corrupted[15] ^= 1
  for (const bytes of [corrupted, frame('assistantResponseEvent', { content: 'Hi' }).subarray(0, 15)]) {
    await assert.rejects(async () => { for await (const _ of kiroEvents((async function* () { yield bytes })())) { /* Consume to validate. */ } },
      e => e.code === 'PROVIDER_RESPONSE')
  }
})
