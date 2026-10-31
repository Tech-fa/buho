import { createHash } from 'node:crypto'
import { readdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

const installersDir = process.argv[2]
const version = process.argv[3]
const notes = process.argv[4] || `Buho ${version}`
const publishedAt = process.argv[5]
const publicBaseUrl = process.argv[6].replace(/\/$/, '')
const prefix = process.argv[7].replace(/^\//, '').replace(/\/$/, '')

const installerPattern = /\.(exe|msi|dmg|appimage|deb)$/i

const platformFor = (file) => {
  const lower = file.toLowerCase()
  if (lower.endsWith('.exe') || lower.endsWith('.msi')) return { platform: 'windows', name: 'Windows' }
  if (lower.endsWith('.dmg')) return { platform: 'macos', name: 'macOS' }
  return { platform: 'linux', name: 'Linux' }
}

const files = (await readdir(installersDir)).filter(file => installerPattern.test(file)).sort()
if (files.length === 0) {
  console.error(`No installers found in ${installersDir}`)
  process.exit(1)
}

const downloads = []
for (const file of files) {
  const bytes = await readFile(path.join(installersDir, file))
  const sha256 = createHash('sha256').update(bytes).digest('hex')
  downloads.push({
    ...platformFor(file),
    file,
    url: `${publicBaseUrl}/${prefix}/${version}/${encodeURIComponent(file)}`,
    sha256,
  })
}

const manifest = { version, publishedAt, notes, downloads }
await writeFile(path.join(installersDir, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
console.log(JSON.stringify(manifest, null, 2))
