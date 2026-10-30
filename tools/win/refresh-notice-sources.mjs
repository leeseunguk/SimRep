// Maintainer-only: refresh audited source snapshots; ordinary staging is offline.
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
const tools = path.dirname(fileURLToPath(import.meta.url))
const dir = path.join(tools, 'notice-sources')
fs.mkdirSync(dir, { recursive: true })
const urls = Object.fromEntries(['MIT', 'Apache-2.0', 'BSD-2-Clause', 'BSD-3-Clause', 'ISC', '0BSD', 'Unlicense', 'Zlib', 'MPL-2.0', 'Unicode-3.0', 'CC0-1.0', 'MIT-0', 'BlueOak-1.0.0', 'Python-2.0'].map(id => [`spdx-${id}`, `https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/${id}.txt`]))
Object.assign(urls, {
  'codex-license': 'https://raw.githubusercontent.com/openai/codex/rust-v0.164.0/LICENSE',
  'codex-notice': 'https://raw.githubusercontent.com/openai/codex/rust-v0.164.0/NOTICE',
  'protobuf-license': 'https://raw.githubusercontent.com/bufbuild/protobuf-es/v2.15.0/LICENSE',
  'protobuf-varint': 'https://raw.githubusercontent.com/bufbuild/protobuf-es/v2.15.0/packages/protobuf/src/wire/varint.ts',
  'senpi-license': 'https://raw.githubusercontent.com/code-yeongyu/senpi/main/LICENSE',
  'undici-license': 'https://raw.githubusercontent.com/nodejs/undici/v5.26.5/LICENSE',
  'msvc-license': 'https://visualstudio.microsoft.com/wp-content/uploads/2021/09/Visual-C-Runtime-2015-2022-License-1.docx',
  'msvc-2026-license': 'https://visualstudio.microsoft.com/wp-content/uploads/2025/10/Visual-C-V14-License-Redistributable_and_Runtime_ENU.docx',
  'ripgrep-license': 'https://raw.githubusercontent.com/BurntSushi/ripgrep/15.2.0/LICENSE-MIT',
  'ripgrep-unlicense': 'https://raw.githubusercontent.com/BurntSushi/ripgrep/15.2.0/UNLICENSE',
  'ripgrep-pcre2-license': 'https://raw.githubusercontent.com/PCRE2Project/pcre2/pcre2-10.45/LICENCE.md',
  'webview2-license': 'https://raw.githubusercontent.com/wravery/webview2-rs/edc2caf886175ccaebe86078c9cfe1ae2a187328/LICENSE',
  'webview2-macros-license': 'https://raw.githubusercontent.com/wravery/webview2-rs/dffa41a8a46d3f5565eefbff2de57d38d399f158/LICENSE',
  'aws-sdk-license': 'https://raw.githubusercontent.com/aws/aws-sdk-js-v3/e5988ecad1aded47fa75f5e18c9cc00ddcb608de/LICENSE',
  'aws-sdk-notice': 'https://raw.githubusercontent.com/aws/aws-sdk-js-v3/e5988ecad1aded47fa75f5e18c9cc00ddcb608de/codegen/NOTICE',
  'lambda-invoke-store-license': 'https://raw.githubusercontent.com/awslabs/aws-lambda-invoke-store/4b2c7152b3f48975059585a09a59c8007ab008fa/LICENSE',
  'lambda-invoke-store-notice': 'https://raw.githubusercontent.com/awslabs/aws-lambda-invoke-store/4b2c7152b3f48975059585a09a59c8007ab008fa/NOTICE',
  'data-uri-to-buffer-license': 'https://raw.githubusercontent.com/TooTallNate/node-data-uri-to-buffer/85cd8c854aefbf1bb636789d80364cfac8ea1583/README.md',
  'proxy-agent-negotiate-license': 'https://raw.githubusercontent.com/TooTallNate/proxy-agents/b7e5f7ccce1a3ac5b339cc4c587974e8989cbc16/packages/proxy-agent/LICENSE',
  'standardwebhooks-license': 'https://raw.githubusercontent.com/standard-webhooks/standard-webhooks/b4d2c14fc5b4ccff3ff271e3b087dff812254c59/libraries/LICENSE',
  'alloc-stdlib-license': 'https://raw.githubusercontent.com/dropbox/rust-alloc-no-stdlib/0a81fd6928ea3b33c8cd484aa4575d50ffb98012/LICENSE',
})
const manifestPath = path.join(dir, 'manifest.json')
// Keep separately maintained, checksummed SDK-embedded source snapshots.
const manifest = fs.existsSync(manifestPath) ? JSON.parse(fs.readFileSync(manifestPath, 'utf8')) : {}
// Optional source IDs permit refreshing a reviewed subset without moving other snapshots.
for (const [id, url] of Object.entries(urls)) {
  if (process.argv.length > 2 && !process.argv.slice(2).includes(id)) continue
  const response = await fetch(url, { signal: AbortSignal.timeout(30000) })
  if (!response.ok) throw new Error(`${url}: ${response.status}`)
  let text
  if (url.endsWith('.docx')) {
    const docx = path.join(dir, `${id}.docx`)
    fs.writeFileSync(docx, Buffer.from(await response.arrayBuffer()))
    const result = spawnSync('python3', [path.join(tools, 'extract-docx-notice.py'), docx], { encoding: 'utf8', timeout: 10000 })
    if (result.status !== 0) throw new Error(result.stderr)
    text = result.stdout
    fs.unlinkSync(docx)
  } else text = await response.text()
  if (id === 'protobuf-varint') {
    const end = text.indexOf('*/')
    if (end < 0 || !text.slice(0, end).includes('Redistribution and use')) throw new Error('Missing protobuf BSD notice')
    text = text.slice(0, end + 2) + '\n'
  }
  const file = `${id}.txt`
  fs.writeFileSync(path.join(dir, file), text)
  manifest[id] = { url, file, sha256: createHash('sha256').update(text).digest('hex') }
  console.log(`${id}: ${Buffer.byteLength(text)} bytes`)
}
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + '\n')
