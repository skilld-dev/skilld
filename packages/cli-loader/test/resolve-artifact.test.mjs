import assert from 'node:assert/strict'
import test from 'node:test'

import { detectLibc, nativePackage, selectArtifact } from '../src/resolve-artifact.mjs'

test('selects the Linux GNU native artifact', () => {
  assert.equal(
    nativePackage({ platform: 'linux', arch: 'x64', libc: 'gnu' }),
    '@skilld/cli-linux-x64-gnu',
  )
  assert.equal(detectLibc({ getReport: () => ({ header: { glibcVersionRuntime: '2.39' } }) }), 'gnu')
})

test('resolves the executable stored by the native package', () => {
  const requests = []
  const result = selectArtifact({
    arch: 'x64',
    forceWasi: false,
    libc: 'gnu',
    node: '/usr/bin/node',
    platform: 'linux',
  }, (packageName, file) => {
    requests.push([packageName, file])
    return '/native/skilld'
  })

  assert.deepEqual(requests, [['@skilld/cli-linux-x64-gnu', 'bin/skilld']])
  assert.deepEqual(result, { _tag: 'Native', executable: '/native/skilld' })
})

test('falls back to WASIp2 when the native artifact is missing', () => {
  const result = selectArtifact({
    arch: 'x64',
    forceWasi: false,
    libc: 'musl',
    node: '/usr/bin/node',
    platform: 'linux',
  }, (packageName, file) => packageName === '@skilld/cli-wasm32-wasi' ? `/wasm/${file}` : undefined)

  assert.deepEqual(result, {
    _tag: 'Wasi',
    executable: '/usr/bin/node',
    runner: '/wasm/run-component.mjs',
  })
})

test('reports a tagged failure when no artifact is installed', () => {
  const result = selectArtifact({
    arch: 'riscv64',
    forceWasi: false,
    libc: 'gnu',
    node: '/usr/bin/node',
    platform: 'linux',
  }, () => undefined)

  assert.deepEqual(result, {
    _tag: 'Unavailable',
    message: 'UNSUPPORTED_HOST: no skilld CLI artifact is installed',
  })
})
