import assert from 'node:assert/strict'
import { it } from 'vitest'

import { detectLibc, nativePackage, selectArtifact } from '../../loader/resolve-artifact.mjs'

it('maps every supported host to one native package', () => {
  assert.deepEqual([
    nativePackage({ platform: 'darwin', arch: 'arm64' }),
    nativePackage({ platform: 'darwin', arch: 'x64' }),
    nativePackage({ platform: 'linux', arch: 'arm64', libc: 'gnu' }),
    nativePackage({ platform: 'linux', arch: 'arm64', libc: 'musl' }),
    nativePackage({ platform: 'linux', arch: 'x64', libc: 'gnu' }),
    nativePackage({ platform: 'linux', arch: 'x64', libc: 'musl' }),
    nativePackage({ platform: 'win32', arch: 'arm64' }),
    nativePackage({ platform: 'win32', arch: 'x64' }),
  ], [
    '@skilld/cli-darwin-arm64',
    '@skilld/cli-darwin-x64',
    '@skilld/cli-linux-arm64-gnu',
    '@skilld/cli-linux-arm64-musl',
    '@skilld/cli-linux-x64-gnu',
    '@skilld/cli-linux-x64-musl',
    '@skilld/cli-win32-arm64-msvc',
    '@skilld/cli-win32-x64-msvc',
  ])
  assert.equal(detectLibc({ getReport: () => ({ header: { glibcVersionRuntime: '2.39' } }) }), 'gnu')
  assert.equal(detectLibc({ getReport: () => ({ header: {} }) }), 'musl')
})

it('selects only the native executable', async () => {
  const requests = []
  const result = await selectArtifact({
    arch: 'x64',
    libc: 'gnu',
    platform: 'linux',
  }, async (packageName, file) => {
    requests.push([packageName, file])
    return '/native/skilld'
  })

  assert.deepEqual(requests, [['@skilld/cli-linux-x64-gnu', 'bin/skilld']])
  assert.deepEqual(result, { _tag: 'Native', executable: '/native/skilld' })
})

it('reports a tagged failure without a JavaScript fallback', async () => {
  const result = await selectArtifact({
    arch: 'riscv64',
    libc: 'gnu',
    platform: 'linux',
  }, async () => undefined)

  assert.deepEqual(result, {
    _tag: 'Unavailable',
    message: 'UNSUPPORTED_HOST: no skilld CLI artifact is installed for linux riscv64',
  })
})
