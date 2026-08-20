#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { constants } from 'node:os'
import { dirname, join } from 'node:path'

import { detectLibc, selectArtifact } from '../loader/resolve-artifact.mjs'

const require = createRequire(import.meta.url)

function resolvePackage(packageName, file) {
  return Promise.resolve()
    .then(() => join(dirname(require.resolve(`${packageName}/package.json`)), file))
    .catch((error) => {
      if (error?.code === 'MODULE_NOT_FOUND' || error?.code === 'ERR_PACKAGE_PATH_NOT_EXPORTED')
        return undefined
      throw error
    })
}

async function main() {
  const artifact = await selectArtifact({
    arch: process.arch,
    libc: process.platform === 'linux' ? detectLibc() : undefined,
    platform: process.platform,
  }, resolvePackage)

  if (artifact._tag === 'Unavailable') {
    process.stderr.write(`${artifact.message}\n`)
    process.exitCode = 2
    return
  }

  const child = spawn(artifact.executable, process.argv.slice(2), {
    cwd: process.cwd(),
    env: process.env,
    shell: false,
    stdio: 'inherit',
  })

  for (const signal of ['SIGINT', 'SIGTERM'])
    process.on(signal, () => child.kill(signal))

  child.on('error', (error) => {
    process.stderr.write(`SERVICE_UNAVAILABLE: ${error.message}\n`)
    process.exitCode = 2
  })
  child.on('exit', (code, signal) => {
    process.exitCode = code ?? 128 + (constants.signals[signal] ?? 0)
  })
}

main().catch((error) => {
  process.stderr.write(`SERVICE_UNAVAILABLE: ${error.message}\n`)
  process.exitCode = 2
})
