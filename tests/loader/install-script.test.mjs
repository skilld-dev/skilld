import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { arch, platform, tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'

import { afterEach, it } from 'vitest'

import { buildReleaseManifest, generateReleaseKey, signReleaseManifest } from '../../scripts/release/release-signing.mjs'

const execFileAsync = promisify(execFile)
const temporaryDirectories = []
const supported = platform() === 'linux' && arch() === 'x64'

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { force: true, recursive: true })))
})

async function temporaryDirectory(prefix) {
  const path = await mkdtemp(join(tmpdir(), prefix))
  temporaryDirectories.push(path)
  return path
}

/**
 * Runs install.sh against a local release directory. Only `download` changes:
 * it copies from that directory instead of fetching over HTTPS.
 *
 * Every run gets its own `HOME`, so no test can write to a real shell profile.
 */
async function install({ tamperSignature = false, tamperBinary = false, publicKey, home, shell = '/bin/sh', installDir, env = {} } = {}) {
  const root = await temporaryDirectory('skilld-install-')
  const release = join(root, 'release')
  await mkdir(release)
  const libc = (await execFileAsync('sh', ['-c', 'ls /lib/ld-musl-* >/dev/null 2>&1 && echo musl || echo gnu'])).stdout.trim()
  const asset = `skilld-cli-linux-x64-${libc}`
  const binary = '#!/bin/sh\necho "skilld 9.0.0"\n'
  const key = generateReleaseKey()
  const manifest = buildReleaseManifest('9.0.0', [{ name: asset, bytes: Buffer.from(binary) }])
  const signature = signReleaseManifest(tamperSignature ? manifest.replace('9.0.0', '9.0.1') : manifest, key.seed)
  await writeFile(join(release, 'skilld-release.txt'), manifest)
  await writeFile(join(release, 'skilld-release.sig'), `${signature}\n`)
  await writeFile(join(release, asset), tamperBinary ? `${binary}# changed\n` : binary)

  const script = (await readFile('install.sh', 'utf8'))
    .replace('__SKILLD_RELEASE_PUBLIC_KEY__', publicKey ?? key.publicKey)
    .replace(/download\(\) \{[\s\S]*?\n\}\n/, `download() {\n  cp "${release}/$(basename "$1")" "$2"\n}\n`)
  await writeFile(join(root, 'install.sh'), script)
  const target = installDir ?? join(root, 'bin')
  const runEnv = { ...process.env, SKILLD_INSTALL_DIR: target, HOME: home ?? await temporaryDirectory('skilld-home-'), SHELL: shell }
  delete runEnv.ZDOTDIR
  delete runEnv.XDG_CONFIG_HOME
  delete runEnv.SKILLD_NO_MODIFY_PATH
  Object.assign(runEnv, env)
  const result = await execFileAsync('sh', [join(root, 'install.sh')], { env: runEnv })
    .then(output => ({ ok: true, ...output }), error => ({ ok: false, ...error }))
  return { result, installDir: target }
}

/**
 * Starts a fresh shell that reads one profile, the way a new terminal does.
 * Bash reads ~/.bashrc by itself when stdin is a socket, as Node's is, so
 * `--norc` keeps that to the one explicit read.
 */
function newShell(shell, home, profile, command) {
  return execFileAsync(shell, [...(shell === 'bash' ? ['--norc'] : []), '-c', `. "${profile}"; ${command}`], {
    env: { HOME: home, PATH: '/usr/bin:/bin' },
  })
}

it.runIf(supported)('installs a signed release and writes the standalone marker', async () => {
  const { result, installDir } = await install()

  assert.ok(result.ok, result.stderr)
  assert.equal(result.stderr, '')
  assert.equal((await execFileAsync(join(installDir, 'skilld'), ['--version'])).stdout, 'skilld 9.0.0\n')
  assert.deepEqual(JSON.parse(await readFile(join(installDir, 'skilld-install.json'), 'utf8')), { channel: 'standalone' })
})

it.runIf(supported)('refuses a manifest signature that does not verify', async () => {
  const { result, installDir } = await install({ tamperSignature: true })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /signature is invalid/)
  await assert.rejects(readFile(join(installDir, 'skilld')))
})

it.runIf(supported)('refuses a release signed by another key', async () => {
  const { result } = await install({ publicKey: generateReleaseKey().publicKey })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /signature is invalid/)
})

it.runIf(supported)('refuses a binary that does not match its signed digest', async () => {
  const { result, installDir } = await install({ tamperBinary: true })

  assert.equal(result.ok, false)
  assert.match(result.stderr, /does not match its digest/)
  await assert.rejects(readFile(join(installDir, 'skilld')))
})

it.runIf(supported)('puts skilld on PATH for new bash shells, once', async () => {
  const home = await temporaryDirectory('skilld-home-')
  const first = await install({ home, shell: '/bin/bash' })
  const second = await install({ home, shell: '/bin/bash', installDir: first.installDir })

  assert.ok(second.result.ok, second.result.stderr)
  assert.match(first.result.stdout, /Open a new terminal/)
  const { stdout } = await newShell('bash', home, join(home, '.bashrc'), 'skilld --version; printf "%s" "$PATH"')
  const [version, path] = stdout.split('\n')
  assert.equal(version, 'skilld 9.0.0')
  assert.equal(path.split(':').filter(entry => entry === first.installDir).length, 1)
})

it.runIf(supported)('puts skilld on PATH through .profile for other shells', async () => {
  const home = await temporaryDirectory('skilld-home-')
  const { result } = await install({ home, shell: '/bin/dash' })

  assert.ok(result.ok, result.stderr)
  const { stdout } = await newShell('sh', home, join(home, '.profile'), 'skilld --version')
  assert.equal(stdout, 'skilld 9.0.0\n')
})

it.runIf(supported)('leaves shell profiles alone when SKILLD_NO_MODIFY_PATH=1', async () => {
  const home = await temporaryDirectory('skilld-home-')
  const { result, installDir } = await install({ home, env: { SKILLD_NO_MODIFY_PATH: '1' } })

  assert.ok(result.ok, result.stderr)
  assert.match(result.stdout, new RegExp(`Add ${installDir} to your PATH`))
  assert.deepEqual(await readdir(home), [])
})
