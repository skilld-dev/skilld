import assert from 'node:assert/strict'
import { it } from 'vitest'

import { detectLibc, nativePackage, selectArtifact } from '../src/resolve-artifact.mjs'

it('selects the Linux GNU native artifact', () => {
  assert.equal(
    nativePackage({ platform: 'linux', arch: 'x64', libc: 'gnu' }),
    '@skilld/cli-linux-x64-gnu',
  )
  assert.equal(detectLibc({ getReport: () => ({ header: { glibcVersionRuntime: '2.39' } }) }), 'gnu')
})

it('resolves the executable stored by the native package', () => {
  const requests = []
  const result = selectArtifact({
    arch: 'x64',
    libc: 'gnu',
    platform: 'linux',
  }, (packageName, file) => {
    requests.push([packageName, file])
    return '/native/skilld'
  })

  assert.deepEqual(requests, [['@skilld/cli-linux-x64-gnu', 'bin/skilld']])
  assert.deepEqual(result, { _tag: 'Native', executable: '/native/skilld' })
})

it('reports a tagged failure when no artifact is installed', () => {
  const result = selectArtifact({
    arch: 'riscv64',
    libc: 'gnu',
    platform: 'linux',
  }, () => undefined)

  assert.deepEqual(result, {
    _tag: 'Unavailable',
    message: 'UNSUPPORTED_HOST: no skilld CLI artifact is installed',
  })
})
