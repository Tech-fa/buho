import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, chmodSync, cpSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const zeros = '0000000000000000000000000000000000000000'

function git(repo, args, env = {}) {
  return execFileSync('git', args, {
    cwd: repo,
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Buho Test',
      GIT_AUTHOR_EMAIL: 'buho@example.com',
      GIT_COMMITTER_NAME: 'Buho Test',
      GIT_COMMITTER_EMAIL: 'buho@example.com',
      ...env,
    },
  }).trim()
}

function fixture(repo) {
  mkdirSync(path.join(repo, 'src-tauri'), { recursive: true })
  mkdirSync(path.join(repo, 'scripts'), { recursive: true })
  writeFileSync(path.join(repo, 'package.json'), '{\n  "name": "buho",\n  "version": "0.1.0"\n}\n')
  writeFileSync(
    path.join(repo, 'package-lock.json'),
    '{\n  "name": "desktop-shell",\n  "version": "0.1.0",\n  "packages": {\n    "": {\n      "version": "0.1.0"\n    },\n    "node_modules/left-pad": {\n      "version": "9.9.9"\n    }\n  }\n}\n',
  )
  writeFileSync(
    path.join(repo, 'src-tauri/Cargo.toml'),
    '[package]\nname = "desktop-shell"\nversion = "0.1.0"\n\n[dependencies]\nserde = "1"\n',
  )
  writeFileSync(
    path.join(repo, 'src-tauri/Cargo.lock'),
    '[[package]]\nname = "other"\nversion = "9.9.9"\n\n[[package]]\nname = "desktop-shell"\nversion = "0.1.0"\n',
  )
  writeFileSync(
    path.join(repo, 'src-tauri/tauri.conf.json'),
    '{\n  "productName": "Buho",\n  "version": "0.1.0"\n}\n',
  )
  cpSync(path.join(here, 'set-buho-version.mjs'), path.join(repo, 'scripts/set-buho-version.mjs'))
  cpSync(path.join(here, 'pre-push'), path.join(repo, 'scripts/pre-push'))
  chmodSync(path.join(repo, 'scripts/pre-push'), 0o755)
}

function initRepo() {
  const repo = mkdtempSync(path.join(tmpdir(), 'buho-pre-push-'))
  git(repo, ['init', '-q'])
  fixture(repo)
  git(repo, ['add', '.'])
  git(repo, ['commit', '-q', '-m', 'start'])
  return repo
}

function runHook(repo, lines) {
  return execFileSync('bash', [path.join(repo, 'scripts/pre-push'), 'origin', 'origin'], {
    cwd: repo,
    input: `${lines.join('\n')}\n`,
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Buho Test',
      GIT_AUTHOR_EMAIL: 'buho@example.com',
      GIT_COMMITTER_NAME: 'Buho Test',
      GIT_COMMITTER_EMAIL: 'buho@example.com',
    },
  })
}

test('a version tag updates the version files and waits for a second push', () => {
  const repo = initRepo()
  git(repo, ['tag', 'v1.2.3'])
  const sha = git(repo, ['rev-parse', 'v1.2.3'])
  assert.throws(() => runHook(repo, [`refs/tags/v1.2.3 ${sha} refs/tags/v1.2.3 ${zeros}`]), /Push v1\.2\.3 again/)
  assert.equal(JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8')).version, '1.2.3')
  assert.match(readFileSync(path.join(repo, 'src-tauri/Cargo.toml'), 'utf8'), /version = "1\.2\.3"/)
  assert.equal(JSON.parse(readFileSync(path.join(repo, 'src-tauri/tauri.conf.json'), 'utf8')).version, '1.2.3')
  const lock = readFileSync(path.join(repo, 'src-tauri/Cargo.lock'), 'utf8')
  assert.match(lock, /name = "desktop-shell"\nversion = "1\.2\.3"/)
  assert.match(lock, /name = "other"\nversion = "9\.9\.9"/)
  const packageLock = readFileSync(path.join(repo, 'package-lock.json'), 'utf8')
  assert.match(packageLock, /"version": "9\.9\.9"/)
  assert.equal(git(repo, ['rev-parse', 'HEAD']), git(repo, ['rev-parse', 'v1.2.3^{commit}']))
  assert.match(git(repo, ['log', '-1', '--format=%s']), /Set Buho version to 1\.2\.3/)

  const again = git(repo, ['rev-parse', 'v1.2.3'])
  assert.equal(runHook(repo, [`refs/tags/v1.2.3 ${again} refs/tags/v1.2.3 ${zeros}`]), '')
})

test('a push without a version tag is left alone', () => {
  const repo = initRepo()
  const sha = git(repo, ['rev-parse', 'HEAD'])
  assert.equal(runHook(repo, [`refs/heads/main ${sha} refs/heads/main ${zeros}`]), '')
  assert.equal(JSON.parse(readFileSync(path.join(repo, 'package.json'), 'utf8')).version, '0.1.0')
})

test('a tag that is already on the remote is not moved', () => {
  const repo = initRepo()
  git(repo, ['tag', 'v1.2.3'])
  const sha = git(repo, ['rev-parse', 'v1.2.3'])
  assert.throws(
    () => runHook(repo, [`refs/tags/v1.2.3 ${sha} refs/tags/v1.2.3 ${sha}`]),
    /already on the remote/,
  )
  assert.equal(git(repo, ['rev-parse', 'HEAD']), git(repo, ['rev-parse', 'v1.2.3^{commit}']))
})
