import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'

import { afterEach, it } from 'vitest'

import {
  independentPackagePublishDecision,
  nativePackageSpecs,
  nativeReleaseMatrix,
  npmTagForVersion,
  packagePublishDecision,
  verifyNativePackages,
  verifyPackedNativePackage,
  verifyReleaseVersions,
} from '../../scripts/release/native-packages.mjs'

const execFileAsync = promisify(execFile)
const temporaryDirectories = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, {
    force: true,
    recursive: true,
  })))
})

it('accepts one executable for every declared native package', async () => {
  const root = await temporaryDirectory()
  for (const spec of nativePackageSpecs)
    await writeNativePackage(root, spec)

  const verified = await verifyNativePackages(root)

  assert.deepEqual(
    verified.map(result => result.packageName),
    nativePackageSpecs.map(spec => spec.packageName),
  )
})

it('rejects a native package when its executable mode was lost', async () => {
  const root = await temporaryDirectory()
  for (const spec of nativePackageSpecs)
    await writeNativePackage(root, spec)
  const spec = nativePackageSpecs.find(candidate => candidate.os === 'linux')
  await chmod(join(root, spec.directory, 'bin', spec.executable), 0o644)

  await assert.rejects(
    verifyNativePackages(root),
    /must be executable/,
  )
})

it('rejects a binary built for another CPU', async () => {
  const root = await temporaryDirectory()
  for (const spec of nativePackageSpecs)
    await writeNativePackage(root, spec)
  const spec = nativePackageSpecs.find(candidate => candidate.directory === 'cli-linux-x64-gnu')
  await writeFile(
    join(root, spec.directory, 'bin', spec.executable),
    executableBytes('elf', 'arm64'),
  )

  await assert.rejects(
    verifyNativePackages(root),
    /ELF binary for x64/,
  )
})

it('accepts the executable from a packed platform package', async () => {
  const root = await temporaryDirectory()
  const output = join(root, 'packed')
  const spec = nativePackageSpecs.find(candidate => candidate.directory === 'cli-linux-x64-gnu')
  await writeNativePackage(root, spec)
  await writeFile(join(root, spec.directory, 'LICENSE'), 'MIT\n')
  await mkdir(output)
  const { stdout } = await execFileAsync('npm', [
    'pack',
    join(root, spec.directory),
    '--json',
    '--pack-destination',
    output,
  ])
  const [{ filename }] = JSON.parse(stdout)

  const verified = await verifyPackedNativePackage(join(output, filename), spec)

  assert.equal(verified.packageName, spec.packageName)
})

it('keeps v3 packages aligned and versions the protocol independently', async () => {
  const root = await temporaryDirectory()
  await writeReleaseManifests(root, '3.0.0-alpha.2', '2.4.0')

  const versions = await verifyReleaseVersions(root, 'v3.0.0-alpha.2')

  assert.deepEqual(versions, {
    protocol: { name: 'skilld-protocol', version: '2.4.0' },
    sharedVersion: '3.0.0-alpha.2',
  })
})

it('publishes beta versions under the beta npm tag', () => {
  assert.equal(npmTagForVersion('3.0.0-beta.0'), 'beta')
})

it('publishes stable versions under the latest npm tag', () => {
  assert.equal(npmTagForVersion('3.0.0'), 'latest')
})

it('disables GCC outline atomics for the ARM64 musl build', () => {
  const release = nativeReleaseMatrix()
  const arm64Musl = release.include.find(item => item.target === 'aarch64-unknown-linux-musl')

  assert.equal(arm64Musl.cflags, '-mno-outline-atomics')
})

it('rejects a release when Harness has a different v3 version', async () => {
  const root = await temporaryDirectory()
  await writeReleaseManifests(root, '3.0.0-alpha.2', '2.4.0')
  await writeFile(join(root, 'packages/harness/package.json'), JSON.stringify({
    name: '@skilld/harness',
    version: '3.0.0-alpha.1',
  }))

  await assert.rejects(
    verifyReleaseVersions(root, 'v3.0.0-alpha.2'),
    /Shared v3 package versions differ/,
  )
})

it('skips an independently versioned package that already exists', () => {
  const decision = independentPackagePublishDecision({
    packageName: 'skilld-protocol',
    response: {
      body: { name: 'skilld-protocol', version: '2.4.0' },
      status: 200,
    },
    version: '2.4.0',
  })

  assert.deepEqual(decision, {
    _tag: 'Skip',
    packageName: 'skilld-protocol',
    version: '2.4.0',
  })
})

it('publishes an independently versioned package only after an exact miss', () => {
  const decision = independentPackagePublishDecision({
    packageName: 'skilld-protocol',
    response: { status: 404 },
    version: '2.4.0',
  })

  assert.deepEqual(decision, {
    _tag: 'Publish',
    packageName: 'skilld-protocol',
    version: '2.4.0',
  })
})

it('fails an independent package lookup on registry errors', () => {
  assert.throws(
    () => independentPackagePublishDecision({
      packageName: 'skilld-protocol',
      response: { status: 503 },
      version: '2.4.0',
    }),
    /HTTP 503/,
  )
})

it('fails when the registry returns another package version', () => {
  assert.throws(
    () => independentPackagePublishDecision({
      packageName: 'skilld-protocol',
      response: {
        body: { name: 'skilld-protocol', version: '2.3.0' },
        status: 200,
      },
      version: '2.4.0',
    }),
    /different package version/,
  )
})

it('skips a shared package that a partial release already published', () => {
  const decision = packagePublishDecision({
    packageName: '@skilld/cli-linux-x64-gnu',
    response: {
      body: { name: '@skilld/cli-linux-x64-gnu', version: '3.0.0-alpha.1' },
      status: 200,
    },
    version: '3.0.0-alpha.1',
  })

  assert.deepEqual(decision, {
    _tag: 'Skip',
    packageName: '@skilld/cli-linux-x64-gnu',
    version: '3.0.0-alpha.1',
  })
})

async function temporaryDirectory() {
  const directory = await mkdtemp(join(tmpdir(), 'skilld-native-package-'))
  temporaryDirectories.push(directory)
  return directory
}

async function writeNativePackage(root, spec) {
  const directory = join(root, spec.directory)
  const executable = join(directory, 'bin', spec.executable)
  const manifest = {
    name: spec.packageName,
    version: '3.0.0-test.0',
    os: [spec.os],
    cpu: [spec.cpu],
    files: [`bin/${spec.executable}`],
    publishConfig: spec.executableMode
      ? { access: 'public', executableFiles: [`bin/${spec.executable}`] }
      : { access: 'public' },
  }
  if (spec.libc)
    manifest.libc = [spec.libc]

  await mkdir(join(directory, 'bin'), { recursive: true })
  await writeFile(join(directory, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`)
  await writeFile(executable, executableBytes(spec.format, spec.cpu))
  if (spec.executableMode)
    await chmod(executable, 0o755)
}

async function writeReleaseManifests(root, sharedVersion, protocolVersion) {
  const manifests = [
    ['package.json', { name: 'skilld', version: sharedVersion }],
    ['packages/harness/package.json', { name: '@skilld/harness', version: sharedVersion }],
    ['packages/protocol/package.json', { name: 'skilld-protocol', version: protocolVersion }],
    ...nativePackageSpecs.map(spec => [
      `packages/${spec.directory}/package.json`,
      { name: spec.packageName, version: sharedVersion },
    ]),
  ]
  for (const [file, manifest] of manifests) {
    await mkdir(join(root, file, '..'), { recursive: true })
    await writeFile(join(root, file), JSON.stringify(manifest))
  }
}

function executableBytes(format, cpu) {
  if (format === 'elf') {
    const bytes = Buffer.alloc(20)
    Buffer.from([0x7F, 0x45, 0x4C, 0x46, 2, 1, 1, 0]).copy(bytes)
    bytes.writeUInt16LE(cpu === 'arm64' ? 0xB7 : 0x3E, 18)
    return bytes
  }
  if (format === 'mach-o') {
    const bytes = Buffer.alloc(8)
    Buffer.from([0xCF, 0xFA, 0xED, 0xFE]).copy(bytes)
    bytes.writeUInt32LE(cpu === 'arm64' ? 0x0100000C : 0x01000007, 4)
    return bytes
  }

  const bytes = Buffer.alloc(72)
  bytes.write('MZ')
  bytes.writeUInt32LE(64, 0x3C)
  bytes.write('PE\0\0', 64, 'binary')
  bytes.writeUInt16LE(cpu === 'arm64' ? 0xAA64 : 0x8664, 68)
  return bytes
}
