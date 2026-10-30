// Native components nested inside the pinned Windows Codex npm payload.
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'

export function providerNotices(root, sources) {
  const location = 'ai/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc'
  const dir = path.join(root, location)
  const voice = path.join(dir, 'codex-resources/voice')
  const read = file => JSON.parse(fs.readFileSync(file, 'utf8'))
  const definitions = {
    libffi: ['MIT', ['libffi.txt']],
    pcre2: ['BSD-3-Clause WITH PCRE2-exception', ['PCRE2.md', 'sljit.txt']],
    zlib: ['Zlib', ['zlib.txt']],
    opus: ['BSD-3-Clause', ['Opus.txt']],
    glib: ['LGPL-2.1-or-later', ['LGPL-2.1.txt']],
    'proxy-libintl': ['LGPL-2.0-or-later', ['proxy-libintl.txt']],
    gstreamer: ['LGPL-2.1-or-later', ['LGPL-2.1.txt']],
    'gst-plugins-base': ['LGPL-2.1-or-later', ['LGPL-2.1.txt']],
    'gst-plugins-good': ['LGPL-2.1-or-later', ['LGPL-2.1.txt']],
  }
  const metadata = read(path.join(voice, 'sources.json'))
  const records = metadata.sources.filter(source => source.role === 'native-library').map(source => {
    const definition = definitions[source.name]
    if (!definition || !source.version || !source.sha256) throw new Error(`Unmapped Codex voice library: ${source.name}`)
    const [license, files] = definition
    return {
      kind: 'provider-native', name: source.name, version: source.version, license,
      location: `${location}/codex-resources/voice (${source.name})`,
      texts: [
        { source: `${source.url} (source archive SHA256 ${source.sha256})`, text: fs.readFileSync(path.join(voice, 'NOTICE.md'), 'utf8') },
        ...files.map(file => ({ source: `${location}/codex-resources/voice/licenses/${file}`, text: fs.readFileSync(path.join(voice, 'licenses', file), 'utf8') })),
      ],
    }
  })
  for (const source of metadata.bundled_sources) {
    if (source.name !== 'gvdb' || !source.revision) throw new Error(`Unmapped embedded voice source: ${source.name}`)
    records.push({ kind: 'provider-native', name: 'GVDB', version: source.revision, license: 'LGPL-2.1-or-later', location: `${location}/codex-resources/voice (GVDB in GLib)`, texts: [{ source: `${location}/codex-resources/voice/licenses/LGPL-2.1.txt`, text: fs.readFileSync(path.join(voice, 'licenses/LGPL-2.1.txt'), 'utf8') }] })
  }
  const crt = read(path.join(voice, 'windows-crt.json'))
  if (crt.version !== '14.50.35719') throw new Error('Unmapped Codex voice CRT version')
  records.push({ kind: 'provider-native', name: 'Microsoft Visual C++ Runtime (Codex voice)', version: crt.version, license: 'LicenseRef-Microsoft-Visual-Cpp-v14-2026', location: `${location}/codex-resources/voice/bin/vcruntime140.dll`, texts: [{ source: sources['msvc-2026-license'].url, text: sources['msvc-2026-license'].text }] })
  const rg = spawnSync(path.join(dir, 'codex-path/rg.exe'), ['--version'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8', timeout: 10000 })
  if (rg.status !== 0 || !/^ripgrep 15\.2\.0\b/m.test(rg.stdout) || !/PCRE2 10\.45\b/.test(rg.stdout)) throw new Error('Unmapped bundled ripgrep/PCRE2 version')
  for (const component of [
    { name: 'ripgrep', version: '15.2.0', license: 'MIT OR Unlicense', ids: ['ripgrep-license', 'ripgrep-unlicense'] },
    { name: 'PCRE2 (ripgrep)', version: '10.45', license: 'BSD-3-Clause WITH PCRE2-exception', ids: ['ripgrep-pcre2-license'] },
  ]) records.push({ kind: 'provider-native', name: component.name, version: component.version, license: component.license, location: `${location}/codex-path/rg.exe (${component.name})`, texts: component.ids.map(id => ({ source: sources[id].url, text: sources[id].text })) })
  if (records.some(record => record.texts.some(file => !file.text.trim()))) throw new Error('Empty provider native notice')
  return records
}
