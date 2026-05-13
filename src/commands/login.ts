/**
 * `skilld login` — authenticate with skilld.dev.
 *
 * Picks a flow based on env:
 *   - `ACTIONS_ID_TOKEN_REQUEST_TOKEN` set → GHA OIDC exchange.
 *   - `--device` or no `DISPLAY`/`BROWSER` env → RFC 8628 device flow.
 *   - Otherwise → PKCE loopback.
 */

import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { runDeviceFlow } from '../auth/device-flow.ts'
import { isGhaOidcAvailable, runOidcExchange } from '../auth/oidc.ts'
import { runPkceFlow } from '../auth/pkce-flow.ts'
import { saveSession } from '../auth/store.ts'
import { getRegistryBase } from '../registry/client.ts'
import { track } from '../telemetry.ts'
import { version } from '../version.ts'

function shouldUseDevice(force: boolean): boolean {
  if (force)
    return true
  if (process.env.BROWSER)
    return false
  if (process.platform === 'darwin' || process.platform === 'win32')
    return false
  return !process.env.DISPLAY && !process.env.WAYLAND_DISPLAY
}

export const loginCommandDef = defineCommand({
  meta: { name: 'login', description: 'Authenticate with skilld.dev' },
  args: {
    device: { type: 'boolean', description: 'Use RFC 8628 device flow' },
  },
  async run({ args }) {
    const registryBase = getRegistryBase()

    if (isGhaOidcAvailable()) {
      const spin = p.spinner()
      spin.start('Exchanging GitHub Actions OIDC token')
      const tokens = await runOidcExchange({ registryBase })
      spin.stop(`Authenticated as @${tokens.login} (oidc)`)
      await saveSession({
        login: tokens.login,
        accessToken: tokens.accessToken,
        expiresAt: tokens.expiresAt,
        tokens: { accessToken: tokens.accessToken },
      })
      track({ event: 'auth-flow', surface: 'cli:auth', flow: 'oidc' })
      process.exit(0)
    }

    if (shouldUseDevice(!!args.device)) {
      const tokens = await runDeviceFlow({
        registryBase,
        cliVersion: version,
        onUserCode: ({ userCode, verificationUri }) => {
          p.log.info(`Visit ${styleText('cyan', verificationUri)} and enter ${styleText('bold', userCode)}`)
        },
      })
      await saveSession({
        login: tokens.login,
        accessToken: tokens.accessToken,
        refreshToken: tokens.refreshToken,
        expiresAt: tokens.expiresAt,
        tokens: { accessToken: tokens.accessToken, refreshToken: tokens.refreshToken },
      })
      track({ event: 'auth-flow', surface: 'cli:auth', flow: 'device' })
      p.log.success(`Logged in as @${tokens.login}`)
      process.exit(0)
    }

    p.log.info('Opening browser to authenticate…')
    const tokens = await runPkceFlow({ registryBase, cliVersion: version })
    await saveSession({
      login: tokens.login,
      accessToken: tokens.accessToken,
      refreshToken: tokens.refreshToken,
      expiresAt: tokens.expiresAt,
      tokens: { accessToken: tokens.accessToken, refreshToken: tokens.refreshToken },
    })
    track({ event: 'auth-flow', surface: 'cli:auth', flow: 'pkce' })
    p.log.success(`Logged in as @${tokens.login}`)
    // Node's global fetch keep-alive pool + telemetry fire-and-forget leave
    // sockets ref'd; force exit so we don't wait for the 4s idle timeout.
    process.exit(0)
  },
})
