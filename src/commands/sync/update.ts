import type { OptimizeModel } from '../../agent/index.ts'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { loadSession, peekMarker, updateMarker } from '../../auth/store.ts'
import { promptForAgent, resolveAgent } from '../../cli/agent-prompt.ts'
import { sharedArgs } from '../../cli/args.ts'
import { renderDigest } from '../../cli/digest-render.ts'
import { isInteractive } from '../../cli/env.ts'
import { getInstalledGenerators, introLine } from '../../cli/intro.ts'
import { readConfig } from '../../core/config.ts'
import { resolveSkillName } from '../../core/prefix.ts'
import { COMMA_OR_WHITESPACE_RE } from '../../core/regex.ts'
import { getProjectState } from '../../core/skills.ts'
import { createRegistryClient } from '../../registry/client.ts'
import { syncCommand } from '../sync.ts'
import { exportPortablePrompts } from './portable.ts'

async function renderChangesDigest(): Promise<void> {
  const session = await loadSession()
  if (!session || session.scheme === 'env')
    return
  const marker = peekMarker()
  const client = createRegistryClient({ session })
  const changes = await client.my.changes({ since: marker?.lastDigestAt }).catch(() => [])
  if (changes.length === 0)
    return
  renderDigest(changes)
  updateMarker({ lastDigestAt: new Date().toISOString() })
}

export const updateCommandDef = defineCommand({
  meta: { name: 'update', description: 'Update outdated skills' },
  args: {
    package: {
      type: 'positional',
      description: 'Package(s) to update (space or comma-separated). Without args, syncs all outdated.',
      required: false,
    },
    background: {
      type: 'boolean',
      alias: 'b',
      description: 'Run in background (detached process, non-interactive)',
      default: false,
    },
    ...sharedArgs,
  },
  async run({ args }) {
    const cwd = process.cwd()

    if (args.background) {
      const { spawn } = await import('node:child_process')
      const updateArgs = ['update', ...(args.package ? [args.package] : []), ...(args.agent ? ['--agent', args.agent] : []), ...(args.model ? ['--model', args.model as string] : [])]
      const child = spawn(process.execPath, [process.argv[1]!, ...updateArgs], {
        cwd,
        detached: true,
        stdio: 'ignore',
      }) as import('node:child_process').ChildProcess
      child.unref()
      return
    }

    const silent = !isInteractive()

    let agent = resolveAgent(args.agent)
    if (!agent) {
      agent = await promptForAgent()
      if (!agent)
        return
    }

    if (agent === 'none') {
      const state = await getProjectState(cwd)
      const packages = args.package
        ? Array.from(
            new Set([args.package, ...((args as any)._ || [])].flatMap(s => s.split(COMMA_OR_WHITESPACE_RE)).map(s => s.trim()).filter(Boolean)),
            s => resolveSkillName(s),
          ).filter((s): s is string => s !== null)
        : state.outdated.map(s => s.packageName || s.name)
      if (packages.length === 0) {
        if (!silent)
          p.log.success('All skills up to date')
        return
      }
      for (const pkg of packages)
        await exportPortablePrompts(pkg, { force: args.force, agent: 'none' })
      return
    }

    const config = readConfig()
    const state = await getProjectState(cwd)

    if (!silent) {
      const generators = getInstalledGenerators()
      p.intro(introLine({ state, generators, modelId: config.model, agentId: config.agent || agent || undefined }))
    }

    if (args.package) {
      const raw = [...new Set([args.package, ...((args as any)._ || [])].flatMap(s => s.split(COMMA_OR_WHITESPACE_RE)).map(s => s.trim()).filter(Boolean))]
      const packages: string[] = []
      for (const r of raw) {
        const name = resolveSkillName(r)
        if (!name) {
          p.log.warn(`Cannot update ${styleText('cyan', r)}: curator/collection inputs are not addressable here.`)
          continue
        }
        packages.push(name)
      }
      if (packages.length === 0)
        return
      return syncCommand(state, {
        packages,
        global: args.global,
        agent,
        model: (args.model as OptimizeModel | undefined) || (silent ? config.model : undefined),
        yes: args.yes || silent,
        force: args.force,
        debug: args.debug,
        mode: 'update',
      })
    }

    const crateSpecs = state.skills
      .map(s => s.info?.packageName)
      .filter((name): name is string => !!name && name.startsWith('crate:'))
    if (state.outdated.length === 0 && crateSpecs.length === 0) {
      p.log.success('All skills up to date')
      return
    }

    const packages = [
      ...state.outdated.map(s => s.packageName || s.name),
      ...crateSpecs,
    ]
    await syncCommand(state, {
      packages,
      global: args.global,
      agent,
      model: (args.model as OptimizeModel | undefined) || (silent ? config.model : undefined),
      yes: args.yes || silent,
      force: args.force,
      debug: args.debug,
      mode: 'update',
    })

    if (!silent)
      await renderChangesDigest()
  },
})
