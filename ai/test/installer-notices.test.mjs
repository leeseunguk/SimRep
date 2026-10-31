import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import os from 'node:os'
import { firstPartyNotice, npmNotices } from '../../tools/win/installer-notices.mjs'

const upstreamLicense = readFileSync(new URL('../../tools/win/notice-sources/standardwebhooks-license.txt', import.meta.url), 'utf8')

function fixture(t, pkg, files = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), 'simrep-notices-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const dir = path.join(root, 'example')
  mkdirSync(dir)
  writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'example', version: '1.0.0', license: 'MIT', ...pkg }))
  for (const [name, text] of Object.entries(files)) writeFileSync(path.join(dir, name), text)
  return { root, dir }
}

test('missing package text without a mapped source fails even with author metadata', t => {
  const { root } = fixture(t, { author: { name: 'Example Author' } })
  assert.throws(() => npmNotices(root), /No full licence text: example@1\.0\.0/)
})

test('version-mapped standardwebhooks preserves upstream copyright, not metadata author', t => {
  const { root } = fixture(t, { name: 'standardwebhooks', version: '1.1.1', author: 'Standard Webhooks' })
  const [notice] = npmNotices(root)
  assert.equal(notice.packageNoticeFiles, 0)
  assert.equal(notice.name, 'standardwebhooks')
  assert.equal(notice.version, '1.1.1')
  assert.equal(notice.license, 'MIT')
  assert.equal(notice.texts[0].text, upstreamLicense)
  assert.match(notice.texts[0].text, /Copyright \(c\) 2023 Svix/)
  assert.equal(notice.texts[0].source, 'https://raw.githubusercontent.com/standard-webhooks/standard-webhooks/b4d2c14fc5b4ccff3ff271e3b087dff812254c59/libraries/LICENSE')
  const updated = fixture(t, { name: 'standardwebhooks', version: '1.1.2', author: 'Standard Webhooks' })
  assert.throws(() => npmNotices(updated.root), /No full licence text: standardwebhooks@1\.1\.2/)
})

test('installed dependencies have attributed, nonempty source notices', () => {
  const packages = npmNotices(fileURLToPath(new URL('../node_modules', import.meta.url)))
  assert.ok(packages.length > 0)
  for (const pkg of packages) {
    assert.ok(pkg.name && pkg.version && pkg.license, pkg.location)
    assert.ok(pkg.texts.length > 0, pkg.location)
    for (const file of pkg.texts) assert.ok(file.source && file.text.trim().length > 0, pkg.location)
  }
})

test('legacy dual-licence separators become SPDX OR expressions preserving shipped texts', t => {
  const apache = readFileSync(new URL('../../tools/win/notice-sources/aws-sdk-license.txt', import.meta.url), 'utf8')
  const { root } = fixture(t, { license: 'MIT / Apache-2.0' }, { 'LICENSE-MIT': upstreamLicense, 'LICENSE-APACHE': apache })
  const [notice] = npmNotices(root)
  assert.equal(notice.license, 'MIT OR Apache-2.0')
  assert.deepEqual(notice.texts.map(file => file.text).sort(), [apache, upstreamLicense].sort())
})

test('complete CC0 notices are preserved and truncated waivers fail closed', t => {
  const cc0 = readFileSync(new URL('../../tools/win/notice-sources/spdx-CC0-1.0.txt', import.meta.url), 'utf8')
  const { root } = fixture(t, { license: 'CC0-1.0' }, { LICENSE: cc0 })
  assert.equal(npmNotices(root)[0].texts[0].text, cc0)
  const truncated = fixture(t, { license: 'CC0-1.0' }, { LICENSE: cc0.slice(0, cc0.indexOf('4. Limitations and Disclaimers')) })
  assert.throws(() => npmNotices(truncated.root), /No full licence text: example@1\.0\.0/)
})

test('licence labels and short excerpts fail even with author metadata', t => {
  const { root } = fixture(t, { author: 'Example Author' }, { LICENSE: 'MIT' })
  assert.throws(() => npmNotices(root), /No full licence text: example@1\.0\.0/)
  const excerpt = fixture(t, { author: 'Example Author' }, { LICENSE: 'Permission is hereby granted. THE SOFTWARE IS PROVIDED AS IS.' })
  assert.throws(() => npmNotices(excerpt.root), /No full licence text: example@1\.0\.0/)
})

test('unknown licence without full text and empty notice files fail closed', t => {
  const { root } = fixture(t, { license: 'LicenseRef-Unknown', author: 'Example Author' })
  assert.throws(() => npmNotices(root), /No full licence text: example@1\.0\.0/)
  const empty = fixture(t, { author: 'Example Author' }, { NOTICE: '' })
  assert.throws(() => npmNotices(empty.root), /Empty or binary notice/)
})

test('nested dependency instances are not skipped and all notice files survive', t => {
  const { root, dir } = fixture(t, { author: 'Example Author' }, { LICENSE: upstreamLicense, NOTICE: 'Copyright Example Author' })
  const nested = path.join(dir, 'node_modules', 'nested')
  mkdirSync(nested, { recursive: true })
  writeFileSync(path.join(nested, 'package.json'), JSON.stringify({ name: 'nested', version: '2.0.0', license: 'MIT' }))
  assert.throws(() => npmNotices(root), /nested@2\.0\.0/)
  writeFileSync(path.join(nested, 'LICENSE'), upstreamLicense)
  const packages = npmNotices(root)
  assert.equal(packages.length, 2)
  assert.ok(packages[0].texts.some(file => file.source.endsWith('/NOTICE')))
  assert.equal(packages[1].texts[0].text, upstreamLicense)
  assert.ok(packages[0].texts.some(file => file.source.endsWith('/LICENSE') && file.text === upstreamLicense))
})

test('first-party installer notice is exactly the shipped root licence', () => {
  const notice = firstPartyNotice()
  assert.equal(notice.kind, 'first-party')
  assert.equal(notice.location, 'LICENSE')
  assert.equal(notice.license, 'PolyForm-Noncommercial-1.0.0')
  assert.deepEqual(notice.texts, [{ source: 'LICENSE', text: readFileSync(new URL('../../LICENSE', import.meta.url), 'utf8') }])
})

test('first-party packaging check rejects an omitted licence resource', t => {
  const { root } = fixture(t, {})
  mkdirSync(path.join(root, 'app/src-tauri'), { recursive: true })
  writeFileSync(path.join(root, 'app/src-tauri/tauri.conf.json'), JSON.stringify({ bundle: { licenseFile: '../../LICENSE', resources: {} } }))
  assert.throws(() => firstPartyNotice(root), /First-party LICENSE must ship/)
})
