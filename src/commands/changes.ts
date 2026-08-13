import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { renderChangesDigest } from './sync/changes-digest.ts'

export const changesCommandDef = defineCommand({
  meta: { name: 'changes', description: 'Show watched skill changes' },
  async run() {
    p.intro('skilld changes')
    const result = await renderChangesDigest(true).catch((error) => {
      p.log.error(`Failed to load changes: ${error instanceof Error ? error.message : String(error)}`)
      process.exitCode = 1
      return null
    })
    if (result === 'auth-required') {
      p.log.error('Not logged in. Run `skilld login` first.')
      process.exitCode = 1
    }
  },
})
