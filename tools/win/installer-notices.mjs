// Generate redistribution notices from the actual staged payload, never licence labels alone.
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { providerNotices } from './provider-notices.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const sourcesDir = path.join(root, 'tools/win/notice-sources')
const hash = text => createHash('sha256').update(text).digest('hex')
const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''))
const relative = (base, file) => path.relative(base, file).split(path.sep).join('/')
const noticeName = /^(licen[cs]e|copying|copyright|notice)([._-].*)?$/i
function readNotice(file) {
  const bytes = fs.readFileSync(file)
  try { return new TextDecoder('utf-8', { fatal: true }).decode(bytes) }
  catch { return new TextDecoder('windows-1252').decode(bytes) }
}

export function noticeFiles(dir, base = dir) {
  const result = []
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    if (entry.name === 'node_modules' || entry.name === '.git') continue
    const file = path.join(dir, entry.name)
    if (entry.isDirectory()) result.push(...noticeFiles(file, base))
    else if (entry.isFile() && (noticeName.test(entry.name) || /(?:^|\/)licen[cs]es\/[^/]+\.(txt|md|rst)$/i.test(relative(base, file)))) {
      const text = readNotice(file)
      if (!text.trim() || text.includes('\0')) throw new Error(`Empty or binary notice: ${file}`)
      result.push({ source: relative(base, file), text })
    }
  }
  return result
}

export function npmPackages(dir) {
  if (!fs.existsSync(dir)) throw new Error(`Missing installed dependency tree: ${dir}`)
  const result = []
  function visit(modules) {
    for (const entry of fs.readdirSync(modules, { withFileTypes: true })) {
      if (entry.name.startsWith('.')) continue
      const file = path.join(modules, entry.name)
      if (entry.isSymbolicLink()) throw new Error(`Dependency links are not a staged package: ${file}`)
      if (!entry.isDirectory()) continue
      if (entry.name.startsWith('@')) {
        for (const child of fs.readdirSync(file)) packageDir(path.join(file, child))
      } else packageDir(file)
    }
  }
  function packageDir(dir) {
    const file = path.join(dir, 'package.json')
    if (!fs.existsSync(file)) throw new Error(`Unidentified package: ${dir}`)
    const pkg = readJson(file)
    if (!pkg.name || !pkg.version) throw new Error(`Missing package identity: ${file}`)
    result.push({ ...pkg, dir })
    const nested = path.join(dir, 'node_modules')
    if (fs.existsSync(nested)) visit(nested)
  }
  visit(dir)
  return result.sort((a, b) => a.dir.localeCompare(b.dir, 'en'))
}

function sources() {
  const manifest = readJson(path.join(sourcesDir, 'manifest.json'))
  for (const [id, entry] of Object.entries(manifest)) {
    entry.text = fs.readFileSync(path.join(sourcesDir, entry.file), 'utf8')
    if (hash(entry.text) !== entry.sha256 || !entry.text.trim()) throw new Error(`Notice source checksum mismatch: ${id}`)
  }
  return manifest
}

// Presence is not enough: reject label-only, short excerpts and link-only files.
function fullText(text) {
  return text.length > 500 && /permission is hereby granted[\s\S]*without warranty|permission is hereby granted[\s\S]*as.is|redistribution and use[\s\S]*disclaim|apache license[\s\S]*terms and conditions[\s\S]*end of terms|mozilla public license[\s\S]*definitions[\s\S]*exhibit|gnu (lesser |library )?general public license[\s\S]*terms and conditions[\s\S]*end of terms|this is free and unencumbered software[\s\S]*as.is|permission to use, copy[\s\S]*disclaims|copyright[\s\S]*provided.*as.is|permission is granted to anyone[\s\S]*this notice may not be removed|prosperity public license[\s\S]*agreement[\s\S]*no liability/i.test(text)
}

const supplements = readJson(path.join(sourcesDir, 'package-map.json'))

function packageNotice(pkg, kind, location, src) {
  const license = (typeof pkg.license === 'string' ? pkg.license : pkg.license?.type)?.replaceAll('/', ' OR ').replace(/\s+/g, ' ').trim()
  if (!license) throw new Error(`Missing SPDX expression: ${pkg.name}@${pkg.version}`)
  const files = noticeFiles(pkg.dir)
  const texts = files.map(file => ({ ...file, source: `${location}/${file.source}` }))
  for (const id of supplements[`${pkg.name}@${pkg.version}`] || []) {
    if (!src[id]) throw new Error(`Missing pinned supplemental notice: ${id}`)
    texts.push({ source: src[id].url, text: src[id].text })
  }
  const commercial = /^@anthropic-ai\/claude-agent-sdk(?:-(?:linux|win32|darwin)-(?:x64|arm64)(?:-musl)?)?$/.test(pkg.name) && pkg.version === '0.3.311'
  let fallback = false
  if (!texts.some(file => fullText(file.text)) && !commercial) {
    const ids = license.match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g).filter(id => !['OR', 'AND', 'WITH'].includes(id))
    const authors = pkg.authors || (pkg.author ? [pkg.author] : [])
    const names = authors.map(author => typeof author === 'string' ? author : author.name).filter(Boolean)
    if (!names.length) throw new Error(`No full licence text or author: ${pkg.name}@${pkg.version} (${location})`)
    for (const id of new Set(ids)) {
      const entry = src[`spdx-${id}`]
      if (!entry) throw new Error(`No full licence text or SPDX source: ${pkg.name}@${pkg.version}: ${id}`)
      const text = entry.text.replace(/Copyright \(c\) <year> <copyright holders>/g, `Copyright: ${names.join('; ')}`)
      texts.push({ source: `${entry.url}; copyright attribution from package metadata`, text: `Copyright: ${names.join('; ')}\n\n${text}` })
    }
    fallback = true
  }
  if (commercial && !files.some(file => /Anthropic PBC.*All rights reserved/s.test(file.text))) throw new Error('Missing Anthropic commercial notice')
  if (!texts.length) throw new Error(`No full notice: ${pkg.name}@${pkg.version}`)
  return { kind, name: pkg.name, version: pkg.version, license: commercial ? 'LicenseRef-Anthropic-Commercial' : license, location, packageNoticeFiles: files.length, fallback, texts }
}

export function npmNotices(dir, kind = 'npm', src = sources()) {
  const lock = kind === 'frontend/node_modules' ? readJson(path.join(dir, '../package-lock.json')) : null
  return npmPackages(dir).filter(pkg => {
    if (!lock) return true
    const entry = lock.packages[`node_modules/${relative(dir, pkg.dir)}`]
    if (!entry) throw new Error(`Frontend package absent from lock: ${pkg.dir}`)
    return !entry.dev
  }).map(pkg => packageNotice(pkg, kind, `${kind}/${relative(dir, pkg.dir)}`, src))
}

function cargoNotices(manifest, features, src) {
  const result = spawnSync('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc', '--manifest-path', manifest, '--features', features], { cwd: root, encoding: 'utf8', maxBuffer: 100_000_000, timeout: 180000 })
  if (result.status !== 0) throw new Error(`cargo metadata failed: ${result.stderr}`)
  const metadata = JSON.parse(result.stdout)
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]))
  const entry = metadata.packages.find(pkg => pkg.manifest_path === path.resolve(manifest))
  if (!entry) throw new Error(`Missing Cargo root: ${manifest}`)
  const included = new Set()
  function visit(id) {
    if (included.has(id)) return
    included.add(id)
    // Include build/proc-macro dependencies conservatively as well as linked crates.
    for (const dep of nodes.get(id).deps) visit(dep.pkg)
  }
  visit(entry.id)
  return metadata.packages.filter(pkg => included.has(pkg.id) && pkg.source).map(pkg => packageNotice({ ...pkg, dir: path.dirname(pkg.manifest_path) }, 'cargo', `${pkg.name}@${pkg.version}`, src))
}

function nativeNotices() {
  const runtime = path.join(root, 'app/src-tauri/bundle-runtime')
  const nodeLicense = readNotice(path.join(runtime, 'LICENSE'))
  if (nodeLicense.length < 150000 || !nodeLicense.includes('OpenSSL') || !nodeLicense.includes('V8')) throw new Error('Node complete upstream LICENSE is missing bundled notices')
  const node = spawnSync(path.join(runtime, 'node.exe'), ['--version'], { encoding: 'utf8', timeout: 10000, stdio: ['ignore', 'pipe', 'pipe'] })
  if (node.status !== 0 || node.stdout.trim() !== 'v24.14.1') throw new Error('Staged Node version is not 24.14.1')
  const records = [{ kind: 'runtime', name: 'Node.js (including embedded third parties)', version: '24.14.1', license: 'MIT AND LicenseRef-Node-Bundled', location: 'runtime/node.exe', texts: [{ source: 'runtime/LICENSE', text: nodeLicense }] }]
  const occt = path.join(root, 'app/src-tauri/bundle-occt')
  const manifest = readJson(path.join(occt, 'notice-manifest.json'))
  const covered = new Set()
  for (const pkg of manifest) {
    if (!pkg.name || !pkg.version || !pkg.license || !pkg.files.length || !pkg.notices.length) throw new Error('Incomplete native notice mapping')
    const texts = pkg.notices.map(file => ({ source: `occt/${file}`, text: readNotice(path.join(occt, file)) }))
    if (texts.some(file => !file.text.trim())) throw new Error(`Empty native notice: ${pkg.name}`)
    for (const file of pkg.files) {
      if (!fs.statSync(path.join(occt, file)).isFile()) throw new Error(`Missing native payload: ${file}`)
      if (covered.has(file.toLowerCase())) throw new Error(`Duplicate native mapping: ${file}`)
      covered.add(file.toLowerCase())
    }
    records.push({ kind: 'native', name: pkg.name, version: pkg.version, license: pkg.license, location: pkg.files.join(', '), texts })
  }
  for (const file of fs.readdirSync(occt).filter(file => /\.dll$/i.test(file)).concat(fs.readdirSync(path.join(occt, 'crt')).filter(file => /\.dll$/i.test(file)).map(file => `crt/${file}`))) {
    if (!covered.has(file.toLowerCase())) throw new Error(`Unmapped shipped DLL: ${file}`)
  }
  return records
}

export function generate() {
  const src = sources()
  const records = [
    ...npmNotices(path.join(root, 'ai/node_modules'), 'ai/node_modules', src),
    ...npmNotices(path.join(root, 'app/node_modules'), 'frontend/node_modules', src),
    ...cargoNotices(path.join(root, 'crates/simrep-cli/Cargo.toml'), 'cad', src),
    ...cargoNotices(path.join(root, 'app/src-tauri/Cargo.toml'), 'cad', src),
    ...nativeNotices(),
    ...providerNotices(root, src),
    ...JSON.parse(src['sdk-embedded'].text),
    { kind: 'adapted-source', name: 'SimRep adapted components', version: '0.1.0', license: 'LicenseRef-SimRep-ThirdParty', location: 'licenses/THIRD_PARTY.md', texts: [{ source: 'licenses/THIRD_PARTY.md', text: fs.readFileSync(path.join(root, 'licenses/THIRD_PARTY.md'), 'utf8') }] },
  ]
  for (const record of records) {
    if (!record.name || !record.version || !record.license || !record.texts?.length || record.texts.some(file => !file.text?.trim())) {
      throw new Error(`Incomplete installer notice record: ${record.name}`)
    }
  }
  const unique = [...new Map(records.map(record => [`${record.kind}:${record.location}`, record])).values()]
  unique.sort((a, b) => `${a.kind}:${a.location}`.localeCompare(`${b.kind}:${b.location}`, 'en'))
  const inventory = unique.map(({ texts, ...record }) => ({ ...record, texts: texts.map(({ source, text }) => ({ source, sha256: hash(text), bytes: Buffer.byteLength(text) })) }))
  const text = 'SimRep Windows installer - complete third-party notices\nGenerated from the staged payload and locked Cargo dependency closures.\nSPDX expressions identify terms, not a replacement for the full texts below.\nFallback entries explicitly identify missing package notice files and metadata attribution.\nThis software is based in part on the work of the Independent JPEG Group.\n\n' + unique.map(record => [
    '='.repeat(78), `Package: ${record.name}`, `Version: ${record.version}`, `SPDX: ${record.license}`, `Kind: ${record.kind}`, `Payload: ${record.location}`,
    ...(record.fallback ? ['Package contains no complete licence text in the inventoried notice files; SPDX standard text with package metadata attribution follows.'] : []),
    ...record.texts.map(file => `\n--- ${file.source} ---\n${file.text}`),
  ].join('\n')).join('\n\n') + '\n'
  return { text, manifest: JSON.stringify({ schema_version: 1, target: 'x86_64-pc-windows-msvc', records: inventory }, null, 2) + '\n', counts: Object.fromEntries([...new Set(unique.map(record => record.kind))].map(kind => [kind, unique.filter(record => record.kind === kind).length])) }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const output = path.join(root, 'licenses/THIRD_PARTY_NOTICES_INSTALLER.txt')
  const manifest = path.join(root, 'licenses/THIRD_PARTY_NOTICES_INSTALLER.json')
  const result = generate()
  if (process.argv.includes('--check')) {
    if (fs.readFileSync(output, 'utf8') !== result.text || fs.readFileSync(manifest, 'utf8') !== result.manifest) throw new Error('Installer notices do not match staged payload; regenerate before packaging')
  } else {
    fs.writeFileSync(output, result.text)
    fs.writeFileSync(manifest, result.manifest)
  }
  console.log(JSON.stringify({ counts: result.counts, bytes: Buffer.byteLength(result.text), output, checked: process.argv.includes('--check') }))
}
