// Builds the environment for one opencode run. Pure: the caller supplies the
// parent environment and the credentials, so the isolation rules are testable.
import { join } from 'node:path'

export interface RunEnvInput {
  /** The parent process environment. */
  base: NodeJS.ProcessEnv
  /** The run's temp directory. It becomes HOME and holds the XDG homes. */
  tempHome: string
  config: Record<string, unknown>
  /** The contents of opencode's `auth.json`, or null when there is none. */
  auth: string | null
  extra: Record<string, string>
}

/**
 * opencode reads Skills from its global config, `~/.claude/skills`, and
 * `~/.agents/skills`. The XDG homes and the two disable flags stop opencode
 * from loading them. The Agent can still read the real home with a shell
 * command, and one baseline run read an installed Skill that way. So HOME
 * moves to the temp directory too. Toolchains that resolve from HOME, such as
 * rustup and cargo, keep their real directories. npm and opencode keep their caches.
 * Provider settings and credentials pass in memory, so no secret lands in the
 * temp directory.
 */
export function runEnv(input: RunEnvInput): NodeJS.ProcessEnv {
  const realHome = input.base.HOME ?? ''
  const env: NodeJS.ProcessEnv = {
    ...input.base,
    HOME: input.tempHome,
    RUSTUP_HOME: input.base.RUSTUP_HOME ?? join(realHome, '.rustup'),
    CARGO_HOME: input.base.CARGO_HOME ?? join(realHome, '.cargo'),
    npm_config_cache: input.base.npm_config_cache ?? join(realHome, '.npm'),
    // opencode keeps provider packages here. It holds no Skills.
    XDG_CACHE_HOME: input.base.XDG_CACHE_HOME ?? join(realHome, '.cache'),
    XDG_CONFIG_HOME: join(input.tempHome, 'config'),
    XDG_DATA_HOME: join(input.tempHome, 'data'),
    XDG_STATE_HOME: join(input.tempHome, 'state'),
    OPENCODE_CONFIG_CONTENT: JSON.stringify(input.config),
    OPENCODE_DISABLE_EXTERNAL_SKILLS: '1',
    OPENCODE_DISABLE_CLAUDE_CODE: '1',
    OPENCODE_DISABLE_AUTOUPDATE: '1',
    OPENCODE_DISABLE_SHARE: '1',
    ...input.extra,
  }
  delete env.OPENCODE_CONFIG
  delete env.OPENCODE_CONFIG_DIR
  if (input.auth !== null)
    env.OPENCODE_AUTH_CONTENT = input.auth
  return env
}
