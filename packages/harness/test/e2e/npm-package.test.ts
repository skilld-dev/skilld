import { createHash } from 'node:crypto'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { gunzipSync, gzipSync } from 'node:zlib'
import { afterEach, vi } from 'vitest'
import { createSkillHarness } from '../../src/index.ts'
import { createFakeHarness, createFakeSandboxProvider, skillSource } from '../support/fakes.ts'

function tarEntry(path: string, content: string, type = '0'): Buffer {
  const body = Buffer.from(content)
  const header = Buffer.alloc(512)
  header.write(path, 0, 100, 'utf8')
  header.write('0000644\0', 100, 8, 'ascii')
  header.write('0000000\0', 108, 8, 'ascii')
  header.write('0000000\0', 116, 8, 'ascii')
  header.write(`${body.byteLength.toString(8).padStart(11, '0')}\0`, 124, 12, 'ascii')
  header.write('00000000000\0', 136, 12, 'ascii')
  header.fill(32, 148, 156)
  header.write(type, 156, 1, 'ascii')
  header.write('ustar\0', 257, 6, 'ascii')
  const checksum = [...header].reduce((total, byte) => total + byte, 0)
  header.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148, 8, 'ascii')
  const padding = Buffer.alloc((512 - body.byteLength % 512) % 512)
  return Buffer.concat([header, body, padding])
}

function archive(type = '0') {
  return gzipSync(Buffer.concat([
    tarEntry('package/package.json', '{"name":"remote-package","version":"1.2.3"}\n', type),
    Buffer.alloc(1024),
  ]))
}

function repositoryArchive(gitHead: string) {
  return gzipSync(Buffer.concat([
    tarEntry(`remote-package-${gitHead}/src/index.ts`, 'export const value = 1\n'),
    Buffer.alloc(1024),
  ]))
}

function registryFetch(tarball: Buffer, integrity: string) {
  const packument = {
    'dist-tags': { latest: '1.2.3' },
    'versions': {
      '1.2.3': {
        version: '1.2.3',
        dist: { tarball: 'https://registry.npmjs.org/remote-package/-/remote-package-1.2.3.tgz', integrity },
      },
    },
  }
  return vi.fn()
    .mockResolvedValueOnce(new Response(JSON.stringify(packument), { status: 200 }))
    .mockResolvedValueOnce(new Response(Uint8Array.from(tarball), { status: 200 }))
}

afterEach(() => vi.unstubAllGlobals())

describe('npm package preparation', () => {
  it('verifies and prepares an exact npm package archive', async () => {
    const tarball = archive()
    const integrity = `sha512-${createHash('sha512').update(tarball).digest('base64')}`
    const fetch = registryFetch(tarball, integrity)
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await expect(sandbox.readTextFile({ path: join(workDir, 'input/source/package.json') })).resolves.toContain('1.2.3')
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/remote-package/SKILL.md'),
          content: skillSource('remote-package'),
        })
      },
    })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider(), fetch }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package@latest' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({
      _tag: 'Ok',
      value: {
        sourceAttempts: [
          { source: 'https://registry.npmjs.org/remote-package', status: 'used' },
          { source: expect.stringContaining('remote-package-1.2.3.tgz'), status: 'used' },
        ],
      },
    })
  })

  it('rejects an archive with the wrong integrity before starting an Agent', async () => {
    const tarball = archive()
    const fetch = registryFetch(tarball, 'sha512-AAAAAAAA')
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider(), fetch }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('rejects a linked npm archive entry before starting an Agent', async () => {
    const tarball = archive('2')
    const integrity = `sha512-${createHash('sha512').update(tarball).digest('base64')}`
    const fetch = registryFetch(tarball, integrity)
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider(), fetch }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('rejects more than three npm redirects', async () => {
    const fetchMock = vi.fn(async () => new Response(null, {
      status: 302,
      headers: { location: 'https://registry.npmjs.org/next' },
    }))
    vi.stubGlobal('fetch', fetchMock)
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fetchMock).toHaveBeenCalledTimes(4)
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('requires the strongest declared npm integrity digest', async () => {
    const tarball = archive()
    const integrity = [
      `sha1-${createHash('sha1').update(tarball).digest('base64')}`,
      'sha512-AAAAAAAA',
    ].join(' ')
    const fetch = registryFetch(tarball, integrity)
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider(), fetch }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('rejects an archive with a corrupt tar header', async () => {
    const tarball = archive()
    const uncompressed = Buffer.from(gunzipSync(tarball))
    uncompressed[0] = 'q'.charCodeAt(0)
    const corrupted = gzipSync(uncompressed)
    const integrity = `sha512-${createHash('sha512').update(corrupted).digest('base64')}`
    const fetch = registryFetch(corrupted, integrity)
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider(), fetch }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('stops reading an npm archive after its byte limit', async () => {
    let pulls = 0
    const archiveBody = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulls += 1
        if (pulls > 1_000) {
          controller.close()
          return
        }
        controller.enqueue(Uint8Array.of(1))
      },
    })
    const packument = {
      'dist-tags': { latest: '1.2.3' },
      'versions': {
        '1.2.3': {
          version: '1.2.3',
          dist: {
            tarball: 'https://registry.npmjs.org/remote-package/-/remote-package-1.2.3.tgz',
            integrity: 'sha512-AAAAAAAA',
          },
        },
      },
    }
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(packument), { status: 200 }))
      .mockResolvedValueOnce(new Response(archiveBody, { status: 200 })))
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({
      harness: fake.harness,
      sandbox: createFakeSandboxProvider(),
      outputPolicy: { maxSourceBytes: 8 },
    }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(pulls).toBeLessThan(100)
    expect(fake.capture.starts).toHaveLength(0)
  })

  it('adds a GitHub source pinned by npm gitHead', async () => {
    const gitHead = '1234567890abcdef1234567890abcdef12345678'
    const tarball = archive()
    const repository = repositoryArchive(gitHead)
    const packument = {
      'dist-tags': { latest: '1.2.3' },
      'repository': { type: 'git', url: 'git+https://github.com/skilld-dev/remote-package.git' },
      'versions': {
        '1.2.3': {
          version: '1.2.3',
          gitHead,
          dist: {
            tarball: 'https://registry.npmjs.org/remote-package/-/remote-package-1.2.3.tgz',
            integrity: `sha512-${createHash('sha512').update(tarball).digest('base64')}`,
          },
        },
      },
    }
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(packument), { status: 200 }))
      .mockResolvedValueOnce(new Response(Uint8Array.from(tarball), { status: 200 }))
      .mockResolvedValueOnce(new Response(Uint8Array.from(repository), { status: 200 })))
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({
      async onPrompt({ sandbox, workDir }) {
        await expect(sandbox.readTextFile({ path: join(workDir, 'input/source/repository/src/index.ts') })).resolves.toContain('value = 1')
        await expect(sandbox.readTextFile({ path: join(workDir, 'input/source-manifest.json') })).resolves.toContain('"version": "1.2.3"')
        await sandbox.writeTextFile({
          path: join(workDir, 'skilld-output/remote-package/SKILL.md'),
          content: skillSource('remote-package'),
        })
      },
    })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package@latest' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({
      _tag: 'Ok',
      value: {
        sourceAttempts: [
          { source: 'https://registry.npmjs.org/remote-package', status: 'used' },
          { source: expect.stringContaining('remote-package-1.2.3.tgz'), status: 'used' },
          { source: expect.stringContaining(`tar.gz/${gitHead}`), status: 'used' },
        ],
      },
    })
  })

  it('rejects a registry version that is not exact', async () => {
    const packument = {
      'dist-tags': { latest: 'workspace:*' },
      'versions': {
        'workspace:*': {
          version: 'workspace:*',
          dist: { tarball: 'https://registry.npmjs.org/remote-package/archive.tgz', integrity: 'sha512-AAAA' },
        },
      },
    }
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(new Response(JSON.stringify(packument), { status: 200 })))
    const destinationRoot = await mkdtemp(join(tmpdir(), 'skilld-output-'))
    const fake = createFakeHarness({ onPrompt: async () => {} })

    const result = await createSkillHarness({ harness: fake.harness, sandbox: createFakeSandboxProvider() }).run({
      _tag: 'PackageSkill',
      source: { _tag: 'NpmPackage', spec: 'remote-package' },
      destination: { rootDir: destinationRoot, name: 'remote-package' },
    })

    expect(result).toMatchObject({ _tag: 'Err', error: { _tag: 'SourceUnavailable' } })
    expect(fake.capture.starts).toHaveLength(0)
  })
})
