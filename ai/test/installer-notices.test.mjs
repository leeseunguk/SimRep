import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { npmNotices } from '../../tools/win/installer-notices.mjs'

function fixture(t, pkg, files = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'simrep-notices-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const dir = path.join(root, 'example')
  mkdirSync(dir)
  writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'example', version: '1.0.0', license: 'MIT', ...pkg }))
  for (const [name, text] of Object.entries(files)) writeFileSync(path.join(dir, name), text)
  return { root, dir }
}

test('missing package text uses full SPDX text and metadata attribution', t => {
  const { root } = fixture(t, { author: { name: 'Example Author' } })
  const [notice] = npmNotices(root)
  assert.equal(notice.packageNoticeFiles, 0)
  assert.equal(notice.fallback, true)
  assert.equal(notice.name, 'example')
  assert.equal(notice.version, '1.0.0')
  assert.equal(notice.license, 'MIT')
  assert.match(notice.texts[0].text, /Example Author/)
  assert.ok(notice.texts[0].text.length > 1000)
})

test('legacy dual-licence separators become SPDX OR expressions with both full texts', t => {
  const { root } = fixture(t, { license: 'MIT / Apache-2.0', author: 'Example Author' })
  const [notice] = npmNotices(root)
  assert.equal(notice.license, 'MIT OR Apache-2.0')
  assert.equal(notice.texts.length, 2)
  assert.ok(notice.texts.every(file => file.text.length > 1000))
})

test('licence label or short excerpt without a full text or author fails closed', t => {
  const { root } = fixture(t, {}, { LICENSE: 'MIT' })
  assert.throws(() => npmNotices(root), /No full licence text or author/)
  const excerpt = fixture(t, {}, { LICENSE: 'Permission is hereby granted. THE SOFTWARE IS PROVIDED AS IS.' })
  assert.throws(() => npmNotices(excerpt.root), /No full licence text or author/)
})

test('unknown SPDX fallback and empty notice files fail closed', t => {
  const { root } = fixture(t, { license: 'LicenseRef-Unknown', author: 'Example Author' })
  assert.throws(() => npmNotices(root), /No full licence text or SPDX source/)
  const empty = fixture(t, { author: 'Example Author' }, { NOTICE: '' })
  assert.throws(() => npmNotices(empty.root), /Empty or binary notice/)
})

test('nested dependency instances are not skipped and all notice files survive', t => {
  const { root, dir } = fixture(t, { author: 'Example Author' }, { NOTICE: 'Copyright Example Author' })
  const nested = path.join(dir, 'node_modules', 'nested')
  mkdirSync(nested, { recursive: true })
  writeFileSync(path.join(nested, 'package.json'), JSON.stringify({ name: 'nested', version: '2.0.0', license: 'MIT' }))
  assert.throws(() => npmNotices(root), /nested@2.0.0/)
  writeFileSync(path.join(nested, 'LICENSE'), readFileSync(new URL('../../tools/win/notice-sources/spdx-MIT.txt', import.meta.url)))
  const packages = npmNotices(root)
  assert.equal(packages.length, 2)
  assert.ok(packages[0].texts.some(file => file.source.endsWith('/NOTICE')))
  assert.equal(packages[1].fallback, false)
})
