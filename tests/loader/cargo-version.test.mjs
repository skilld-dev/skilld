import assert from 'node:assert/strict'
import { it } from 'vitest'

import { setCargoWorkspaceVersion } from '../../scripts/release/cargo-version.mjs'

const manifest = `[workspace]
members = ["crates/skilld-core"]

[workspace.package]
version = "3.0.0"
edition = "2024"

[workspace.dependencies]
ed25519-dalek = "3.0.0"
signature = { version = "3.0.0" }
`

it('changes only the workspace package version', () => {
  const next = setCargoWorkspaceVersion(manifest, '3.0.1')

  assert.equal(next, manifest.replace('[workspace.package]\nversion = "3.0.0"', '[workspace.package]\nversion = "3.0.1"'))
  assert.match(next, /ed25519-dalek = "3\.0\.0"/)
  assert.match(next, /signature = \{ version = "3\.0\.0" \}/)
})

it('fails when the manifest has no workspace package version', () => {
  assert.throws(() => setCargoWorkspaceVersion('[workspace.dependencies]\nversion = "3.0.0"\n', '3.0.1'), /\[workspace\.package\] version/)
})
