#!/usr/bin/env node
import { constants } from 'node:os'
import { dirname, join } from 'node:path'
import { createRequire } from 'node:module'
import { spawn } from 'node:child_process'

import { detectLibc, selectArtifact } from '../src/resolve-artifact.mjs'

const require = createRequire(import.meta.url)
const resolvePackage = (packageName, file) => {
  try {
    return join(dirname(require.resolve(`${packageName}/package.json`)), file)
  }
  catch (error) {
    if (error?.code === 'MODULE_NOT_FOUND' || error?.code === 'ERR_PACKAGE_PATH_NOT_EXPORTED')
      return undefined
    throw error
  }
}

const artifact = selectArtifact({
  arch: process.arch,
  forceWasi: process.env.SKILLD_FORCE_WASI === '1',
  libc: process.platform === 'linux' ? detectLibc() : undefined,
  node: process.execPath,
  platform: process.platform,
}, resolvePackage)

if (artifact._tag === 'Unavailable') {
  process.stderr.write(`${artifact.message}\n`)
  process.exitCode = 2
}
else {
  const args = artifact._tag === 'Wasi'
    ? [artifact.runner, ...process.argv.slice(2)]
    : process.argv.slice(2)
  const child = spawn(artifact.executable, args, {
    cwd: process.cwd(),
    env: process.env,
    shell: false,
    stdio: 'inherit',
  })

  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.on(signal, () => child.kill(signal))
  }
  child.on('error', (error) => {
    process.stderr.write(`SERVICE_UNAVAILABLE: ${error.message}\n`)
    process.exitCode = 2
  })
  child.on('exit', (code, signal) => {
    process.exitCode = code ?? 128 + (constants.signals[signal] ?? 0)
  })
}
