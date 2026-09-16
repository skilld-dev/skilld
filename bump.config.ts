import { defineConfig } from 'bumpp'

import { syncCargoVersion } from './scripts/release/cargo-version.mjs'

export default defineConfig({
  commit: 'chore(release): prepare {tag}',
  files: [
    'package.json',
    'packages/harness/package.json',
    'packages/cli-darwin-arm64/package.json',
    'packages/cli-darwin-x64/package.json',
    'packages/cli-linux-arm64-gnu/package.json',
    'packages/cli-linux-arm64-musl/package.json',
    'packages/cli-linux-x64-gnu/package.json',
    'packages/cli-linux-x64-musl/package.json',
    'packages/cli-win32-arm64-msvc/package.json',
    'packages/cli-win32-x64-msvc/package.json',
  ],
  // Cargo files stay out of `files`: a text replace also rewrites matching dependency versions.
  execute: syncCargoVersion,
  noGitCheck: true,
})
