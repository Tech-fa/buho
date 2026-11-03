import { chmod, copyFile, mkdir } from 'node:fs/promises'
import { execSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))

let hooks
try {
  hooks = execSync('git rev-parse --git-path hooks', {
    cwd: path.resolve(here, '..'),
    encoding: 'utf8',
  }).trim()
} catch {
  process.exit(0)
}

const destinationDir = path.resolve(path.resolve(here, '..'), hooks)
await mkdir(destinationDir, { recursive: true })
const destination = path.join(destinationDir, 'pre-push')
await copyFile(path.join(here, 'pre-push'), destination)
await chmod(destination, 0o755)
