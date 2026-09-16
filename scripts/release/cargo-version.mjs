import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'

const WORKSPACE_VERSION = /(\[workspace\.package\]\n(?:[^[\n][^\n]*\n)*?version = ")[^"]+(")/

/**
 * Sets the `[workspace.package]` version and nothing else. A plain text
 * replace also rewrites any dependency that shares the old version number.
 */
export function setCargoWorkspaceVersion(manifest, version) {
  if (!WORKSPACE_VERSION.test(manifest))
    throw new Error('Cargo.toml has no [workspace.package] version.')
  return manifest.replace(WORKSPACE_VERSION, `$1${version}$2`)
}

/** bumpp `execute` hook: sync the Rust workspace, then refresh only its lockfile entries. */
export function syncCargoVersion({ state }) {
  writeFileSync('Cargo.toml', setCargoWorkspaceVersion(readFileSync('Cargo.toml', 'utf8'), state.newVersion))
  execFileSync('cargo', ['update', '--workspace', '--offline'], { stdio: 'inherit' })
}
