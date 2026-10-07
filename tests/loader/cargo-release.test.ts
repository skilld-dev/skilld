import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { it } from 'vitest'
import config from '../../bump.config'

const exec = promisify(execFile)

it('commits the Cargo versions with the npm release without taking unrelated changes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'skilld-release-'))
  try {
    assert(Array.isArray(config.files))
    for (const file of config.files) {
      await mkdir(dirname(join(root, file)), { recursive: true })
      await writeFile(join(root, file), JSON.stringify({ name: 'release-fixture', version: '3.6.6' }))
    }
    await mkdir(join(root, 'crates/fixture/src'), { recursive: true })
    await writeFile(join(root, 'Cargo.toml'), '[workspace]\nmembers = ["crates/fixture"]\nresolver = "3"\n\n[workspace.package]\nversion = "3.6.6"\nedition = "2024"\n')
    await writeFile(join(root, 'crates/fixture/Cargo.toml'), '[package]\nname = "fixture"\nversion.workspace = true\nedition.workspace = true\n')
    await writeFile(join(root, 'crates/fixture/src/lib.rs'), '')
    await exec('cargo', ['generate-lockfile', '--offline'], { cwd: root })
    await writeFile(join(root, 'unrelated.txt'), 'original')
    await exec('git', ['init', '--quiet'], { cwd: root })
    await exec('git', ['config', 'user.name', 'Release test agent'], { cwd: root })
    await exec('git', ['config', 'user.email', 'agent@example.test'], { cwd: root })
    await exec('git', ['add', '.'], { cwd: root })
    await exec('git', ['commit', '--quiet', '-m', 'test: create release fixture'], { cwd: root })
    await writeFile(join(root, 'unrelated.txt'), 'keep outside the release')

    await exec(process.execPath, [
      join(dirname(fileURLToPath(import.meta.resolve('bumpp/package.json'))), 'bin/bumpp.mjs'),
      '--configFilePath', fileURLToPath(new URL('../../bump.config.ts', import.meta.url)),
      '--release', '3.6.7', '--yes', '--no-tag', '--no-push', '--ignore-scripts',
    ], { cwd: root })

    // Read the committed state, rather than the hook's uncommitted working files.
    await exec('git', ['restore', '--source=HEAD', 'Cargo.toml', 'Cargo.lock'], { cwd: root })
    const { stdout } = await exec('cargo', ['metadata', '--format-version', '1', '--no-deps', '--locked', '--offline'], { cwd: root })
    const metadata = JSON.parse(stdout) as { packages: { version: string }[] }
    assert.equal(metadata.packages[0]?.version, '3.6.7')
    const committed = await exec('git', ['show', 'HEAD:package.json'], { cwd: root })
    assert.equal(JSON.parse(committed.stdout).version, '3.6.7')
    const unrelated = await exec('git', ['show', 'HEAD:unrelated.txt'], { cwd: root })
    assert.equal(unrelated.stdout, 'original')
  }
  finally {
    await rm(root, { recursive: true, force: true })
  }
})
