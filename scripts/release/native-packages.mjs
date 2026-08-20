import { execFile } from 'node:child_process'
import { mkdtemp, readdir, readFile, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

export const nativePackageSpecs = Object.freeze([
  nativePackage({ directory: 'cli-darwin-arm64', runner: 'macos-15', target: 'aarch64-apple-darwin', os: 'darwin', cpu: 'arm64', format: 'mach-o' }),
  nativePackage({ directory: 'cli-darwin-x64', runner: 'macos-15-intel', target: 'x86_64-apple-darwin', os: 'darwin', cpu: 'x64', format: 'mach-o' }),
  nativePackage({ directory: 'cli-linux-arm64-gnu', runner: 'ubuntu-24.04-arm', target: 'aarch64-unknown-linux-gnu', os: 'linux', cpu: 'arm64', libc: 'glibc', format: 'elf' }),
  nativePackage({ directory: 'cli-linux-arm64-musl', runner: 'ubuntu-24.04-arm', target: 'aarch64-unknown-linux-musl', os: 'linux', cpu: 'arm64', libc: 'musl', format: 'elf' }),
  nativePackage({ directory: 'cli-linux-x64-gnu', runner: 'ubuntu-24.04', target: 'x86_64-unknown-linux-gnu', os: 'linux', cpu: 'x64', libc: 'glibc', format: 'elf' }),
  nativePackage({ directory: 'cli-linux-x64-musl', runner: 'ubuntu-24.04', target: 'x86_64-unknown-linux-musl', os: 'linux', cpu: 'x64', libc: 'musl', format: 'elf' }),
  nativePackage({ directory: 'cli-win32-arm64-msvc', runner: 'windows-11-arm', target: 'aarch64-pc-windows-msvc', os: 'win32', cpu: 'arm64', format: 'pe' }),
  nativePackage({ directory: 'cli-win32-x64-msvc', runner: 'windows-2025', target: 'x86_64-pc-windows-msvc', os: 'win32', cpu: 'x64', format: 'pe' }),
])

export function nativeReleaseMatrix() {
  return {
    include: nativePackageSpecs.map(spec => ({
      directory: spec.directory,
      executable: spec.executable,
      libc: spec.libc ?? 'none',
      runner: spec.runner,
      target: spec.target,
    })),
  }
}

export async function verifyNativePackages(packagesRoot) {
  return Promise.all(nativePackageSpecs.map(spec => verifyNativePackage(
    join(packagesRoot, spec.directory),
    spec,
  )))
}

export async function verifyNativePackage(directory, spec) {
  const manifest = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'))
  assertEqual(manifest.name, spec.packageName, `${spec.packageName} has the wrong package name`)
  assertEqual(manifest.os, [spec.os], `${spec.packageName} has the wrong operating system`)
  assertEqual(manifest.cpu, [spec.cpu], `${spec.packageName} has the wrong CPU`)
  assertEqual(manifest.libc, spec.libc ? [spec.libc] : undefined, `${spec.packageName} has the wrong libc`)
  assertEqual(manifest.files, [`bin/${spec.executable}`], `${spec.packageName} has the wrong package files`)
  if (spec.executableMode) {
    assertEqual(
      manifest.publishConfig?.executableFiles,
      [`bin/${spec.executable}`],
      `${spec.packageName} does not declare its executable mode`,
    )
  }

  const binDirectory = join(directory, 'bin')
  const entries = await readdir(binDirectory)
  assertEqual(entries, [spec.executable], `${spec.packageName} has unexpected binary files`)
  const binary = join(binDirectory, spec.executable)
  const metadata = await stat(binary)
  if (!metadata.isFile())
    throw new Error(`${spec.packageName} does not contain a binary file`)
  if (spec.executableMode && (metadata.mode & 0o111) === 0)
    throw new Error(`${spec.packageName} binary must be executable`)

  const bytes = await readFile(binary)
  verifyBinaryFormat(bytes, spec)
  return { binary, bytes: bytes.length, packageName: spec.packageName }
}

export async function verifyPackedNativePackage(archive, spec) {
  const temporary = await mkdtemp(join(tmpdir(), 'skilld-packed-native-'))
  return execFileAsync('tar', ['-xzf', archive, '-C', temporary])
    .then(async () => {
      const packageRoot = join(temporary, 'package')
      const files = await listFiles(packageRoot)
      assertEqual(
        files,
        [`bin/${spec.executable}`, 'LICENSE', 'package.json'].sort(),
        `${spec.packageName} packed unexpected files`,
      )
      return verifyNativePackage(packageRoot, spec)
    })
    .finally(() => rm(temporary, { force: true, recursive: true }))
}

export async function verifyPackedNativePackages(archivesRoot) {
  const archives = (await readdir(archivesRoot))
    .filter(file => file.endsWith('.tgz'))
    .map(file => join(archivesRoot, file))
  if (archives.length !== nativePackageSpecs.length)
    throw new Error(`Expected ${nativePackageSpecs.length} packed native packages, found ${archives.length}`)

  const verified = []
  for (const archive of archives) {
    const { stdout } = await execFileAsync('tar', ['-xOzf', archive, 'package/package.json'])
    const manifest = JSON.parse(stdout)
    const spec = nativePackageSpecs.find(candidate => candidate.packageName === manifest.name)
    if (!spec)
      throw new Error(`${basename(archive)} is not a declared native package`)
    verified.push(await verifyPackedNativePackage(archive, spec))
  }

  const actual = verified.map(result => result.packageName).sort()
  const expected = nativePackageSpecs.map(spec => spec.packageName).sort()
  assertEqual(actual, expected, 'Packed native package set is incomplete')
  return verified
}

export async function verifyReleaseVersions(repositoryRoot, tag) {
  const root = JSON.parse(await readFile(join(repositoryRoot, 'package.json'), 'utf8'))
  const harness = JSON.parse(await readFile(join(repositoryRoot, 'packages/harness/package.json'), 'utf8'))
  const protocol = JSON.parse(await readFile(join(repositoryRoot, 'packages/protocol/package.json'), 'utf8'))
  const sharedPackages = [root, harness]
  for (const spec of nativePackageSpecs) {
    sharedPackages.push(JSON.parse(await readFile(
      join(repositoryRoot, 'packages', spec.directory, 'package.json'),
      'utf8',
    )))
  }
  const mismatched = sharedPackages
    .filter(manifest => manifest.version !== root.version)
    .map(manifest => `${manifest.name}@${manifest.version}`)
  if (mismatched.length > 0)
    throw new Error(`Shared v3 package versions differ: ${mismatched.join(', ')}`)
  if (tag !== `v${root.version}`)
    throw new Error(`Release tag ${tag} does not match v${root.version}`)

  return {
    protocol: { name: protocol.name, version: protocol.version },
    sharedVersion: root.version,
  }
}

function nativePackage(spec) {
  const executable = spec.os === 'win32' ? 'skilld.exe' : 'skilld'
  return Object.freeze({
    ...spec,
    executable,
    executableMode: spec.os !== 'win32',
    packageName: `@skilld/${spec.directory}`,
  })
}

function verifyBinaryFormat(bytes, spec) {
  if (spec.format === 'elf') {
    const machine = spec.cpu === 'arm64' ? 0xB7 : 0x3E
    const valid = bytes.length >= 20
      && bytes.subarray(0, 4).equals(Buffer.from([0x7F, 0x45, 0x4C, 0x46]))
      && bytes.readUInt16LE(18) === machine
    if (!valid)
      throw new Error(`${spec.packageName} does not contain an ELF binary for ${spec.cpu}`)
  }
  if (spec.format === 'mach-o') {
    const cpu = spec.cpu === 'arm64' ? 0x0100000C : 0x01000007
    const valid = bytes.length >= 8
      && bytes.subarray(0, 4).equals(Buffer.from([0xCF, 0xFA, 0xED, 0xFE]))
      && bytes.readUInt32LE(4) === cpu
    if (!valid)
      throw new Error(`${spec.packageName} does not contain a Mach-O binary for ${spec.cpu}`)
  }
  if (spec.format === 'pe') {
    const offset = bytes.length >= 64 ? bytes.readUInt32LE(0x3C) : bytes.length
    const machine = spec.cpu === 'arm64' ? 0xAA64 : 0x8664
    const valid = bytes.subarray(0, 2).equals(Buffer.from('MZ'))
      && bytes.subarray(offset, offset + 4).equals(Buffer.from('PE\0\0', 'binary'))
      && bytes.length >= offset + 6
      && bytes.readUInt16LE(offset + 4) === machine
    if (!valid)
      throw new Error(`${spec.packageName} does not contain a PE binary for ${spec.cpu}`)
  }
}

async function listFiles(root) {
  const files = []
  await visit(root)
  return files.sort()

  async function visit(directory) {
    const entries = await readdir(directory, { withFileTypes: true })
    for (const entry of entries) {
      const path = join(directory, entry.name)
      if (entry.isDirectory())
        await visit(path)
      else if (entry.isFile())
        files.push(relative(root, path))
      else
        throw new Error(`Packed native package contains an unsupported file: ${entry.name}`)
    }
  }
}

function assertEqual(actual, expected, message) {
  if (JSON.stringify(actual) !== JSON.stringify(expected))
    throw new Error(message)
}

async function main([command, argument]) {
  if (command === 'matrix') {
    process.stdout.write(JSON.stringify(nativeReleaseMatrix()))
    return
  }
  if (command === 'directories') {
    process.stdout.write(`${nativePackageSpecs.map(spec => spec.directory).join('\n')}\n`)
    return
  }
  if (command === 'verify') {
    await verifyNativePackages(resolve(argument ?? 'packages'))
    return
  }
  if (command === 'verify-packed') {
    await verifyPackedNativePackages(resolve(argument ?? 'artifacts/native-packages'))
    return
  }
  if (command === 'versions') {
    const versions = await verifyReleaseVersions(process.cwd(), argument)
    process.stdout.write(`Shared v3 packages: ${versions.sharedVersion}\n`)
    process.stdout.write(`Independent protocol: ${versions.protocol.version}\n`)
    return
  }
  throw new Error('Expected directories, matrix, verify, verify-packed, or versions')
}

if (resolve(process.argv[1] ?? '') === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  })
}
