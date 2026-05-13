/**
 * `skilld whoami` — print the active login + storage scheme.
 */

import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { loadSession } from '../auth/store.ts'

export const whoamiCommandDef = defineCommand({
  meta: { name: 'whoami', description: 'Show the active skilld.dev session' },
  async run() {
    const session = await loadSession()
    if (!session) {
      p.log.info('Not logged in. Run `skilld login` to authenticate.')
      return
    }
    p.log.message(`Logged in as ${styleText('cyan', `@${session.login}`)} ${styleText('gray', `(${session.scheme})`)}`)
  },
})
