/**
 * `skilld logout` — revoke the active session server-side and clear local credentials.
 * Local state is cleared even if the server revoke fails.
 */

import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { ofetch } from 'ofetch'
import { clearSession, loadSession } from '../auth/store.ts'
import { getRegistryBase } from '../registry/client.ts'

export const logoutCommandDef = defineCommand({
  meta: { name: 'logout', description: 'Sign out of skilld.dev' },
  async run() {
    const session = await loadSession()
    if (!session) {
      p.log.info('Not logged in.')
      return
    }

    await ofetch(`${getRegistryBase()}/cli/logout`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${session.accessToken}` },
    }).catch(() => {})

    await clearSession()
    p.log.success('Logged out.')
  },
})
