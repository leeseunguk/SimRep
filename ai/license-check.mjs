// Inspect npm's installed-package inventory without executing package hooks.
import { spawnSync } from 'node:child_process'
const result = spawnSync(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['query', '*', '--json'], {
  cwd: new URL('.', import.meta.url), encoding: 'utf8', shell: false, maxBuffer: 10_000_000,
})
if (result.status !== 0) throw new Error('npm query failed')
const packages = JSON.parse(result.stdout)
const counts = {}
const exceptions = []
for (const pkg of packages) {
  const license = typeof pkg.license === 'string' ? pkg.license : JSON.stringify(pkg.license) || 'UNSPECIFIED'
  counts[license] = (counts[license] || 0) + 1
  if (!/^(MIT|Apache-2\.0|BSD-2-Clause|BSD-3-Clause|ISC|0BSD|Unlicense|BlueOak-1\.0\.0|Python-2\.0|CC0-1\.0|UNLICENSED)$/.test(license)) {
    exceptions.push({ name: pkg.name, version: pkg.version, license })
  }
}
console.log(JSON.stringify({ packages: packages.length, licences: counts, review: exceptions }, null, 2))
