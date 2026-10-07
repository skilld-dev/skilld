import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

const WORKSPACE_VERSION = /(\[workspace\.package\]\n(?:[^[\n][^\n]*\n)*?version = ")([^"]+)(")/

export function cargoWorkspaceVersion(manifest) {
  const match = manifest.match(WORKSPACE_VERSION)
  if (!match)
    throw new Error('Cargo.toml has no [workspace.package] version.')
  return match[2]
}

/**
 * Sets the `[workspace.package]` version and nothing else. A plain text
 * replace also rewrites any dependency that shares the old version number.
 */
export function setCargoWorkspaceVersion(manifest, version) {
  if (!WORKSPACE_VERSION.test(manifest))
    throw new Error('Cargo.toml has no [workspace.package] version.')
  return manifest.replace(WORKSPACE_VERSION, `$1${version}$3`)
}

/** bumpp `execute` hook: sync the Rust workspace, then refresh only its lockfile entries. */
export function syncCargoVersion(operation) {
  const { state, options } = operation
  const manifest = resolve(options.cwd, 'Cargo.toml')
  const lockfile = resolve(options.cwd, 'Cargo.lock')
  writeFileSync(manifest, setCargoWorkspaceVersion(readFileSync(manifest, 'utf8'), state.newVersion))
  execFileSync('cargo', ['update', '--workspace', '--offline'], { cwd: options.cwd, stdio: 'inherit' })
  // bumpp commits only updatedFiles. Hook-written files must join that commit explicitly.
  operation.update({ updatedFiles: [...state.updatedFiles, manifest, lockfile] })
}
