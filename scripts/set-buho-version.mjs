import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+$/

export function versionIn(text, file) {
  if (file.endsWith('Cargo.toml') || file.endsWith('Cargo.lock')) {
    const match = text.match(/^version = "([^"]+)"/m)
    if (!match) {
      throw new Error(`${file} has no version.`)
    }
    return match[1]
  }
  const match = text.match(/"version"\s*:\s*"([^"]+)"/)
  if (!match) {
    throw new Error(`${file} has no version.`)
  }
  return match[1]
}

export function withVersion(text, file, version) {
  if (!VERSION.test(version)) {
    throw new Error(`Version must look like 1.0.5, received ${version}.`)
  }
  if (file.endsWith('Cargo.toml')) {
    let inPackage = false
    let done = false
    const lines = text.split('\n').map((line) => {
      if (line.startsWith('[')) {
        inPackage = line.trim() === '[package]'
        return line
      }
      if (inPackage && !done && line.startsWith('version = "')) {
        done = true
        return `version = "${version}"`
      }
      return line
    })
    if (!done) {
      throw new Error(`${file} has no package version.`)
    }
    return lines.join('\n')
  }
  if (file.endsWith('Cargo.lock')) {
    let done = false
    const lines = text.split('\n')
    for (let index = 0; index < lines.length - 1; index += 1) {
      if (lines[index] === 'name = "desktop-shell"' && lines[index + 1].startsWith('version = "')) {
        lines[index + 1] = `version = "${version}"`
        done = true
        break
      }
    }
    if (!done) {
      throw new Error(`${file} has no desktop-shell package version.`)
    }
    return lines.join('\n')
  }
  if (file === 'package-lock.json') {
    let replaced = 0
    const lines = text.split('\n').map((line) => {
      if (replaced >= 2 || !line.includes('"version":')) {
        return line
      }
      replaced += 1
      return line.replace(/"version"\s*:\s*"[^"]+"/, `"version": "${version}"`)
    })
    if (replaced !== 2) {
      throw new Error(`${file} does not have the two root version fields.`)
    }
    return lines.join('\n')
  }
  const replaced = text.replace(/"version"\s*:\s*"[^"]+"/, `"version": "${version}"`)
  if (replaced === text) {
    throw new Error(`${file} has no version.`)
  }
  return replaced
}

const FILES = [
  'package.json',
  'package-lock.json',
  'src-tauri/Cargo.toml',
  'src-tauri/Cargo.lock',
  'src-tauri/tauri.conf.json',
]

export function writeVersion(root, version) {
  for (const file of FILES) {
    const full = path.join(root, file)
    const text = readFileSync(full, 'utf8')
    writeFileSync(full, withVersion(text, file, version))
  }
}

function readStdin() {
  return readFileSync(0, 'utf8')
}

function main() {
  const [command, fileOrVersion] = process.argv.slice(2)
  if (command === '--print') {
    process.stdout.write(versionIn(readStdin(), fileOrVersion))
    return
  }
  if (!command) {
    throw new Error('Pass the version, such as 1.0.5.')
  }
  writeVersion(fileOrVersion || process.cwd(), command)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main()
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exit(1)
  }
}
