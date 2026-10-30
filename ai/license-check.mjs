// Check the installed tree, including nested dependencies and native provider notices.
// Generation uses the same fail-closed inventory during Windows runtime staging.
import { fileURLToPath } from 'node:url'
import { npmNotices } from '../tools/win/installer-notices.mjs'
const packages = npmNotices(fileURLToPath(new URL('./node_modules', import.meta.url)))
const counts = {}
for (const pkg of packages) counts[pkg.license] = (counts[pkg.license] || 0) + 1
console.log(JSON.stringify({ packages: packages.length, licences: counts, fullTextVerified: true, spdxFallbacks: packages.filter(pkg => pkg.fallback).map(pkg => `${pkg.name}@${pkg.version}`) }, null, 2))
