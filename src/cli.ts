#!/usr/bin/env node
import type { PackageUsage } from './agent/detect-imports.ts'
import type { AgentType } from './agent/index.ts'
import { existsSync, readFileSync, realpathSync } from 'node:fs'
import { styleText } from 'node:util'
import * as p from '@clack/prompts'
import { defineCommand, runMain } from 'citty'
import pLimit from 'p-limit'
import { join, resolve } from 'pathe'
import { agents, detectImportedPackages, detectInstalledAgents } from './agent/index.ts'
import { promptForAgent, resolveAgent } from './cli/agent-prompt.ts'
import { sharedArgs } from './cli/args.ts'
import { isInteractive, isRunningInsideAgent } from './cli/env.ts'
import { formatStatus, getRepoHint, relativeTime } from './cli/intro.ts'
import { guard, menuLoop } from './cli/menu.ts'
import { hasPrepareHook, suggestPrepareHook } from './cli/prepare-hook.ts'
import { configCommand, configCommandDef } from './commands/config.ts'
import { removeCommand, removeCommandDef } from './commands/remove.ts'
import { infoCommandDef, statusCommand } from './commands/status.ts'
import { runWizard } from './commands/wizard.ts'
import { timedSpinner } from './core/formatting.ts'
import { getProjectState, hasCompletedWizard, isOutdated, readConfig, semverGt } from './core/index.ts'
import { readPackageJsonSafe } from './core/package-json.ts'
import { COMMA_OR_WHITESPACE_RE, VERSION_RANGE_PREFIX_RE } from './core/regex.ts'
import { iterateSkills } from './core/skills.ts'
import { fetchLatestVersion, fetchNpmRegistryMeta } from './sources/index.ts'

import { brandLoader } from './ui.ts'
import { version } from './version.ts'

const STATIC_REGEX_3 = /^[\^~>=<]/

// Suppress node:sqlite ExperimentalWarning (loaded lazily by retriv)
const _emit = process.emit
process.emit = (event: string, ...args: any[]) =>
  event === 'warning' && args[0]?.name === 'ExperimentalWarning' && args[0]?.message?.includes('SQLite')
    ? false
    : _emit.apply(process, [event, ...args])

// ── Deprecation forwarder ──

function deprecatedForwarder(
  oldName: string,
  newName: string,
  loader: () => Promise<any>,
): () => Promise<any> {
  return () => loader().then((cmd: any) => {
    const original = cmd.run
    return defineCommand({
      ...cmd,
      meta: { ...cmd.meta, name: oldName },
      async run(ctx: any) {
        console.warn(styleText('yellow', `⚠ \`skilld ${oldName}\` is deprecated. Use \`skilld ${newName}\` instead.`))
        return original(ctx)
      },
    })
  })
}

// ── Subcommands (lazy-loaded) ──

const SUBCOMMAND_NAMES = ['add', 'eject', 'update', 'info', 'list', 'config', 'remove', 'install', 'uninstall', 'search', 'cache', 'validate', 'assemble', 'setup', 'prepare', 'author', 'publish', 'upload']

// ── Main command ──

const main = defineCommand({
  meta: {
    name: 'skilld',
    version,
    description: 'Curated agent skills for your projects',
  },
  args: {
    agent: sharedArgs.agent,
  },
  subCommands: {
    add: () => import('./commands/sync/add.ts').then(m => m.addCommandDef),
    update: () => import('./commands/sync/update.ts').then(m => m.updateCommandDef),
    info: () => infoCommandDef,
    list: () => import('./commands/list.ts').then(m => m.listCommandDef),
    config: () => configCommandDef,
    remove: () => removeCommandDef,
    install: () => import('./commands/install.ts').then(m => m.installCommandDef),
    prepare: () => import('./commands/prepare.ts').then(m => m.prepareCommandDef),
    uninstall: () => import('./commands/uninstall.ts').then(m => m.uninstallCommandDef),
    search: () => import('./commands/search.ts').then(m => m.searchCommandDef),
    cache: () => import('./commands/cache.ts').then(m => m.cacheCommandDef),
    setup: () => import('./commands/wizard.ts').then(m => m.setupCommandDef),
    login: () => import('./commands/login.ts').then(m => m.loginCommandDef),
    logout: () => import('./commands/logout.ts').then(m => m.logoutCommandDef),
    whoami: () => import('./commands/whoami.ts').then(m => m.whoamiCommandDef),
    pull: () => import('./commands/pull.ts').then(m => m.pullCommandDef),
    // Author group (nested subcommands)
    author: () => import('./commands/author.ts').then(m => m.authorGroupDef),
    // Deprecated forwarders (old top-level commands → skilld author <subcommand>)
    eject: deprecatedForwarder('eject', 'author eject', () => import('./commands/sync/eject.ts').then(m => m.ejectCommandDef)),
    validate: deprecatedForwarder('validate', 'author validate', () => import('./commands/validate.ts').then(m => m.validateCommandDef)),
    assemble: deprecatedForwarder('assemble', 'author assemble', () => import('./commands/assemble.ts').then(m => m.assembleCommandDef)),
    publish: deprecatedForwarder('publish', 'author publish', () => import('./commands/upload.ts').then(m => m.uploadCommandDef)),
    upload: deprecatedForwarder('upload', 'author publish', () => import('./commands/upload.ts').then(m => m.uploadCommandDef)),
  },
  async run({ args }) {
    // Guard: citty always calls parent run() after subcommand dispatch.
    // If a subcommand was invoked, bail out here.
    const firstArg = process.argv[2]
    if (firstArg && !firstArg.startsWith('-') && SUBCOMMAND_NAMES.includes(firstArg))
      return

    const cwd = process.cwd()

    // Bare `skilld` — interactive menu (requires TTY)
    if (!isInteractive()) {
      const state = await getProjectState(cwd)
      const status = formatStatus(state.synced.length, state.outdated.length)
      console.log(`skilld v${version} · ${status}`)
      if (isRunningInsideAgent())
        console.log('Interactive wizard requires a standalone terminal (detected agent session).\nUse `skilld add <pkg>` to add skills non-interactively, or run `npx skilld` in a separate terminal.')
      return
    }

    let currentAgent: AgentType | 'none' | null = resolveAgent(args.agent)

    if (!currentAgent) {
      currentAgent = await promptForAgent()
      if (!currentAgent)
        return
    }

    // No-agent mode: run wizard, then guide them
    if (currentAgent === 'none') {
      if (!hasCompletedWizard()) {
        if (!await runWizard())
          return
      }
      p.log.info(
        'No agent selected - skills export as portable PROMPT_*.md files.\n'
        + `  Run ${styleText('cyan', 'skilld add <pkg>')} to generate prompts for any package.\n`
        + `  Run ${styleText('cyan', 'skilld config')} to set a target agent later.`,
      )
      return
    }

    // After this point, agent is guaranteed to be a real AgentType
    const agent: AgentType = currentAgent

    // Animate brand while bootstrapping + check for updates
    let { state, selfUpdate } = await brandLoader(async () => {
      const config = readConfig()
      const state = await getProjectState(cwd)

      // Run self-update check + unmatched skills NPM check in parallel
      let selfUpdate = null as { latest: string, releasedAt?: string } | null
      const tasks: Promise<void>[] = []

      // Check if skilld itself has a newer version (skip for npx/dlx/bunx)
      const isEphemeral = process.env.npm_command === 'exec'
      if (!isEphemeral) {
        tasks.push(
          fetchNpmRegistryMeta('skilld', version).then((meta) => {
            const latestTag = meta.distTags?.latest
            if (latestTag && semverGt(latestTag.version, version))
              selfUpdate = { latest: latestTag.version, releasedAt: latestTag.releasedAt }
          }).catch(() => {}),
        )
      }

      // For skills not in local deps, check NPM for version updates
      if (state.unmatched.length > 0) {
        const limit = pLimit(5)
        tasks.push(
          Promise.all(state.unmatched.map(skill => limit(async () => {
            const pkgName = skill.info?.packageName || skill.name
            const latest = await fetchLatestVersion(pkgName)
            if (latest && isOutdated(skill, latest)) {
              state.outdated.push({ ...skill, packageName: pkgName, latestVersion: latest })
            }
            else if (latest) {
              state.synced.push({ ...skill, packageName: pkgName, latestVersion: latest })
            }
          }))).then(() => {}),
        )
      }

      await Promise.all(tasks)
      return { config, state, selfUpdate }
    })

    // Show self-update notification
    if (selfUpdate) {
      const released = selfUpdate.releasedAt ? styleText('gray', ` · ${relativeTime(new Date(selfUpdate.releasedAt))}`) : ''
      const binPath = realpathSync(process.argv[1]!)
      const isLocal = binPath.startsWith(resolve(cwd, 'node_modules'))
      const flag = isLocal ? '' : ' -g'
      const cmd = `npx nypm add${flag} skilld@${selfUpdate.latest}`
      p.note(
        `${styleText('gray', version)} → ${styleText(['bold', 'green'], selfUpdate.latest)}${released}\n${styleText('cyan', cmd)}`,
        styleText('yellow', 'Update available'),
      )
    }

    // First time setup or returning with no skills (e.g. cancelled last time)
    if (state.skills.length === 0) {
      if (!hasCompletedWizard()) {
        if (!await runWizard({ agent, showOutro: false }))
          return
      }
      else {
        p.log.step('No skills installed yet - pick some packages to get started.')
      }

      // Transition to project setup
      const pkgJsonPath = join(cwd, 'package.json')
      const projectPkg = readPackageJsonSafe(pkgJsonPath)
      const hasPkgJson = !!projectPkg
      const projectName = projectPkg?.parsed.name as string | undefined
      const projectLabel = projectName
        ? `Generating skills for ${styleText('cyan', projectName)}`
        : 'Generating skills for current directory'
      p.log.step(projectLabel)

      if (!hasPkgJson) {
        p.log.warn('No package.json found - enter npm package names manually.\n  For best results, run skilld inside a JS/TS project directory.')
      }

      if (state.shipped.length > 0) {
        const totalShipped = state.shipped.reduce((sum, s) => sum + s.skills.length, 0)
        const names = state.shipped.map(s => s.packageName).join(', ')
        p.log.info(`${styleText('cyan', `${totalShipped} ready-to-use skill${totalShipped > 1 ? 's' : ''}`)} shipped by your dependencies: ${names}`)
      }

      p.log.info('Tip: Add skills for packages with complex APIs or frequent breaking changes - not every dependency needs one.')

      // Initial setup loop — allow user to go back
      let setupComplete = false
      while (!setupComplete) {
        const shippedOption = state.shipped.length > 0
          ? [{ label: 'Install shipped skills', value: 'shipped' as const, hint: styleText('cyan', `${state.shipped.reduce((sum, s) => sum + s.skills.length, 0)} ready to use`) }]
          : []

        const source = hasPkgJson
          ? await p.select({
              message: 'How should I find packages?',
              options: [
                ...shippedOption,
                { label: 'Scan source files', value: 'imports', hint: 'find actually used imports' },
                { label: 'Use package.json', value: 'deps', hint: `all ${state.deps.size} dependencies` },
                { label: 'Enter manually', value: 'manual' },
                { label: 'Skip for now', value: 'skip', hint: 'add skills later with `skilld add <pkg>`' },
              ],
            })
          : 'manual' as const

        if (p.isCancel(source)) {
          p.cancel('Setup cancelled')
          return
        }

        if (source === 'skip') {
          p.log.info(`Run ${styleText('cyan', 'skilld add <pkg>')} or ${styleText('cyan', 'skilld')} anytime to add skills.`)
          return
        }

        if (source === 'shipped') {
          const { handleShippedSkills: installShipped } = await import('./agent/skill-installer.ts')
          for (const pkg of state.shipped) {
            const version = state.deps.get(pkg.packageName)?.replace(VERSION_RANGE_PREFIX_RE, '') || '0.0.0'
            installShipped(pkg.packageName, version, cwd, agent, false)
            for (const sk of pkg.skills)
              p.log.success(`Installed shipped skill: ${sk.skillName}`)
          }
          state = await getProjectState(cwd)
          continue
        }

        // Get packages based on source
        let selected: string[]

        if (source === 'manual') {
          const input = await p.text({
            message: 'Enter package names (space or comma-separated)',
            placeholder: 'vue nuxt pinia',
          })
          if (p.isCancel(input)) {
            if (!hasPkgJson) {
              p.cancel('Setup cancelled')
              return
            }
            continue
          }
          if (!input) {
            p.log.warn('No packages entered')
            continue
          }
          selected = input.split(COMMA_OR_WHITESPACE_RE).map(s => s.trim()).filter(Boolean)
          if (selected.length === 0) {
            p.log.warn('No valid packages entered')
            continue
          }
        }
        else {
          let usages: PackageUsage[]
          if (source === 'imports') {
            const spinner = timedSpinner()
            spinner.start('Scanning imports...')
            const result = await detectImportedPackages(cwd)

            if (result.packages.length === 0) {
              spinner.stop('No imports found, falling back to package.json')
              usages = Array.from(state.deps.keys(), name => ({ name, count: 0 }))
            }
            else {
              const depSet = new Set(state.deps.keys())
              usages = result.packages.filter(pkg => depSet.has(pkg.name) || pkg.source === 'preset')

              if (usages.length === 0) {
                spinner.stop(`Found ${result.packages.length} imported packages but none match dependencies`)
                usages = result.packages
              }
              else {
                spinner.stop(`Found ${usages.length} imported packages`)
              }
            }
          }
          else {
            usages = Array.from(state.deps.keys(), name => ({ name, count: 0 }))
          }

          // Let user select which packages
          const packages = usages.map(u => u.name)
          if (packages.length === 0) {
            p.log.warn('No packages found')
            continue
          }
          const sourceMap = new Map(usages.map(u => [u.name, u.source]))
          const maxLen = Math.max(...packages.map(n => n.length))
          // Pre-select frameworks and presets (most likely to benefit from skills)
          const preselect = packages.filter((name) => {
            if (sourceMap.get(name) === 'preset')
              return true
            // Common frameworks with complex/changing APIs
            const frameworks = new Set(['vue', 'nuxt', 'react', 'next', 'svelte', '@sveltejs/kit', 'astro', 'solid-js', 'angular', 'typescript', 'vite', 'vitest'])
            return frameworks.has(name)
          })

          const choice = await p.multiselect({
            message: `Select packages (${packages.length} found)`,
            options: packages.map((name) => {
              const ver = state.deps.get(name)?.replace(STATIC_REGEX_3, '') || ''
              const repo = getRepoHint(name, cwd)
              const hint = sourceMap.get(name) === 'preset' ? 'nuxt module' : undefined
              const pad = ' '.repeat(maxLen - name.length + 2)
              const meta = [ver, hint, repo].filter(Boolean).join('  ')
              return { label: meta ? `${name}${pad}${styleText('gray', meta)}` : name, value: name }
            }),
            initialValues: preselect,
          })

          if (p.isCancel(choice)) {
            continue
          }
          if (choice.length === 0) {
            p.log.warn('No packages selected')
            continue
          }
          selected = choice
        }

        // Pass wizard-configured model so sync doesn't re-prompt.
        // Wizard already handled model selection - yes:true auto-resolves.
        const wizardConfig = readConfig()
        const { syncCommand } = await import('./commands/sync.ts')
        await syncCommand(state, {
          packages: selected,
          global: false,
          agent,
          model: wizardConfig.model as import('./agent/index.ts').OptimizeModel | undefined,
          yes: !wizardConfig.skipLlm,
        })
        setupComplete = true
      }

      // Show SKILL.md preview and verification for newly generated skills
      const postState = await getProjectState(cwd)
      const previewSkill = postState.skills[0]
      if (previewSkill) {
        const previewPath = join(cwd, agents[agent].skillsDir, previewSkill.name, 'SKILL.md')
        if (existsSync(previewPath)) {
          const previewContent = readFileSync(previewPath, 'utf-8')
          // eslint-disable-next-line no-control-regex, regexp/no-obscure-range
          const previewLines = previewContent.split('\n').slice(0, 20).join('\n').replace(/\x1B\[[0-?]*[ -/]*[@-~]/g, '').replace(/\x1B\].*?(?:\x07|\x1B\\)/g, '')
          const fileSize = (Buffer.byteLength(previewContent) / 1024).toFixed(1)
          p.note(
            styleText('gray', `${previewLines}\n...`),
            `${agents[agent].skillsDir}/${previewSkill.name}/SKILL.md (${fileSize} KB)`,
          )
        }
      }

      // First-run guidance with agent-specific verification tips
      const agentName = agents[agent].displayName
      const agentInstalled = detectInstalledAgents().includes(agent)
      const verifyTips: Record<string, string> = {
        'claude-code': 'Start a new Claude Code session - skills load automatically.\nOr type /skill-name to invoke a specific skill.',
        'cursor': 'Restart Cursor to pick up new skills.\nSkills appear in Settings > Cursor Rules.',
        'github-copilot': 'Restart your editor to pick up new skills.\nCopilot discovers skills from .github/skills/ at startup.',
        'gemini-cli': 'Start a new Gemini CLI session.\nVerify with /skills list.',
        'codex': 'Start a new Codex session.\nSkills in .agents/skills/ are discovered at startup.',
        'windsurf': 'Restart Windsurf to pick up new skills.\nSkills auto-invoke when their description matches your prompt.',
        'cline': 'Restart your editor. Cline reads skill descriptions at startup.\nFull content loads on-demand when the agent invokes use_skill.',
        'goose': 'Start a new Goose session.\nSkills are discovered automatically at startup.',
        'amp': 'Start a new Amp session.\nReads skill descriptions at startup, full content on invocation.',
        'opencode': 'Start a new OpenCode session.\nSkills are discovered automatically at startup.',
        'roo': 'Restart your editor. Roo reads skill descriptions at startup.',
      }
      const verifyLine = agentInstalled
        ? (verifyTips[agent] ?? '')
        : `Skills are ready in ${agents[agent].skillsDir}/.\n${styleText('gray', `${agentName} was not detected on this machine.\nInstall it to use these skills, or run \`skilld config\` to change agents.`)}`

      // Build a "try it" suggestion that tests skill-specific knowledge
      const firstPkg = previewSkill?.info?.packageName || previewSkill?.name
      const trySuggestion = firstPkg
        ? `\n\n${styleText('cyan', 'Try it:')} ask your agent "What are the gotchas or breaking changes in ${firstPkg}?"`
        : ''

      p.note(
        `${verifyLine}${trySuggestion}\n\n`
        + `Run ${styleText('cyan', 'skilld info')} to see installed skills.\n`
        + `Run ${styleText('cyan', 'skilld')} again to add more, update, or search.`,
        `${agentName} - next steps`,
      )

      // Team advice: suggest prepare hook + lockfile
      try {
        await suggestPrepareHook(cwd)
      }
      catch (err) {
        p.log.warn(`Failed to suggest prepare hook: ${err instanceof Error ? err.message : String(err)}`)
      }
      return
    }

    // Has skills - show status + interactive menu
    const status = formatStatus(state.synced.length, state.outdated.length)
    p.log.info(status)

    let needsPrepareHook = !hasPrepareHook(cwd)
    if (needsPrepareHook) {
      p.log.warn(`${styleText('yellow', 'No prepare hook.')} Skills won't auto-restore on ${styleText('cyan', 'npm install')}.`)
    }

    if (state.shipped.length > 0) {
      const totalSkills = state.shipped.reduce((sum, s) => sum + s.skills.length, 0)
      const names = state.shipped.map(s => s.packageName).join(', ')
      p.log.info(`${styleText('cyan', `${totalSkills} ready-to-use skill${totalSkills > 1 ? 's' : ''}`)} shipped by your dependencies: ${names}`)
    }

    const refreshState = async () => {
      state = await getProjectState(cwd)
    }

    // Main menu — Escape in sub-actions returns to menu via guard()
    await menuLoop({
      message: 'What would you like to do?',
      options: () => {
        const opts: Array<{ label: string, value: string, hint?: string }> = []
        if (state.shipped.length > 0) {
          const total = state.shipped.reduce((sum, s) => sum + s.skills.length, 0)
          opts.push({ label: 'Install shipped skills', value: 'shipped', hint: styleText('cyan', `${total} available`) })
        }
        opts.push({ label: 'Add new skills', value: 'install' })
        if (state.outdated.length > 0) {
          opts.push({ label: 'Update skills', value: 'update', hint: styleText('yellow', `${state.outdated.length} outdated`) })
        }
        if (needsPrepareHook) {
          opts.push({ label: 'Setup prepare hook', value: 'prepare-hook', hint: styleText('yellow', 'recommended') })
        }
        opts.push(
          { label: 'Remove skills', value: 'remove' },
          { label: 'Search docs', value: 'search' },
          { label: 'Info', value: 'info' },
          { label: 'Configure', value: 'config' },
        )
        return opts
      },
      onSelect: async (action) => {
        switch (action) {
          case 'shipped': {
            const allShipped = state.shipped.flatMap(s => s.skills.map(sk => ({ packageName: s.packageName, ...sk })))
            const selected = guard(await p.multiselect({
              message: 'Select shipped skills to install',
              options: allShipped.map(s => ({
                label: s.skillName,
                value: s,
                hint: s.packageName,
              })),
              initialValues: allShipped,
            }))
            if (selected.length === 0)
              return
            const { handleShippedSkills: installShipped } = await import('./agent/skill-installer.ts')
            const seen = new Set<string>()
            for (const s of selected) {
              if (seen.has(s.packageName))
                continue
              seen.add(s.packageName)
              const version = state.deps.get(s.packageName)?.replace(VERSION_RANGE_PREFIX_RE, '') || '0.0.0'
              installShipped(s.packageName, version, cwd, agent, false)
            }
            p.log.success(`Installed ${selected.length} shipped skill${selected.length > 1 ? 's' : ''}`)
            await refreshState()
            return true
          }
          case 'install': {
            const installedNames = new Set([
              ...state.synced.map(s => s.packageName),
              ...state.outdated.map(s => s.packageName),
            ].filter(Boolean) as string[])
            const uninstalledDeps = [...state.deps.keys()].filter(d => !installedNames.has(d))
            const allDepsInstalled = uninstalledDeps.length === 0
            const hasPkgJsonMenu = !!readPackageJsonSafe(join(cwd, 'package.json'))

            const source = hasPkgJsonMenu
              ? guard(await p.select({
                  message: 'How should I find packages?',
                  options: [
                    { label: 'Scan source files', value: 'imports' as const, hint: allDepsInstalled ? 'all installed' : 'find actually used imports', disabled: allDepsInstalled },
                    { label: 'Use package.json', value: 'deps' as const, hint: allDepsInstalled ? 'all installed' : `${uninstalledDeps.length} uninstalled`, disabled: allDepsInstalled },
                    { label: 'Enter manually', value: 'manual' as const },
                  ],
                }))
              : 'manual' as const

            let selected: string[]

            if (source === 'manual') {
              const input = guard(await p.text({
                message: 'Enter package names (space or comma-separated)',
                placeholder: 'vue nuxt pinia',
              }))
              if (!input)
                return
              selected = input.split(COMMA_OR_WHITESPACE_RE).map(s => s.trim()).filter(Boolean)
              if (selected.length === 0)
                return
            }
            else {
              let usages: PackageUsage[]
              if (source === 'imports') {
                const spinner = timedSpinner()
                spinner.start('Scanning imports...')
                const result = await detectImportedPackages(cwd)

                if (result.packages.length === 0) {
                  spinner.stop('No imports found, falling back to package.json')
                  usages = uninstalledDeps.map(name => ({ name, count: 0 }))
                }
                else {
                  const depSet = new Set(state.deps.keys())
                  const matched = result.packages
                    .filter(pkg => depSet.has(pkg.name) || pkg.source === 'preset')
                  const alreadyInstalled = matched.filter(pkg => installedNames.has(pkg.name))
                  usages = matched.filter(pkg => !installedNames.has(pkg.name))

                  if (usages.length === 0) {
                    spinner.stop('All detected imports already have skills')
                    return
                  }
                  else {
                    spinner.stop(`Found ${matched.length} imported packages`)
                    if (alreadyInstalled.length > 0) {
                      p.log.info(`${alreadyInstalled.length} already have skills installed`)
                    }
                  }
                }
              }
              else {
                usages = uninstalledDeps.map(name => ({ name, count: 0 }))
              }

              const packages = usages.map(u => u.name)
              if (packages.length === 0) {
                p.log.warn('No packages found')
                return
              }
              const usageMap = new Map(usages.map(u => [u.name, u]))
              const sourceMap = new Map(usages.map(u => [u.name, u.source]))
              const frameworks = new Set(['vue', 'nuxt', 'react', 'next', 'svelte', '@sveltejs/kit', 'astro', 'solid-js', 'angular', 'typescript', 'vite', 'vitest'])
              const maxLen = Math.max(...packages.map(n => n.length))
              const choice = guard(await p.multiselect({
                message: `Select packages your agent struggles with or that are new to you (${packages.length} found)`,
                options: packages.map((name) => {
                  const ver = state.deps.get(name)?.replace(STATIC_REGEX_3, '') || ''
                  const repo = getRepoHint(name, cwd)
                  const src = sourceMap.get(name)
                  const hint = src === 'preset'
                    ? 'nuxt module'
                    : frameworks.has(name)
                      ? 'framework'
                      : (usageMap.get(name)?.count ?? 0) >= 5
                          ? `${usageMap.get(name)!.count} imports`
                          : undefined
                  const pad = ' '.repeat(maxLen - name.length + 2)
                  const meta = [ver, hint, repo].filter(Boolean).join('  ')
                  return { label: meta ? `${name}${pad}${styleText('gray', meta)}` : name, value: name }
                }),
                initialValues: [],
              }))

              if (choice.length === 0)
                return
              selected = choice
            }

            const { syncCommand: sync } = await import('./commands/sync.ts')
            await sync(state, {
              packages: selected,
              global: false,
              agent,
              yes: false,
            })
            await refreshState()
            return true
          }
          case 'update': {
            if (state.outdated.length === 0) {
              p.log.success('All skills up to date')
              return true
            }
            const selected = guard(await p.multiselect({
              message: 'Select packages to update',
              options: state.outdated.map(s => ({
                label: s.name,
                value: s.packageName || s.name,
                hint: `${s.info?.version ?? 'unknown'} → ${s.latestVersion}`,
              })),
              initialValues: state.outdated.map(s => s.packageName || s.name),
            }))
            if (selected.length === 0)
              return
            const { syncCommand: syncUpdate } = await import('./commands/sync.ts')
            await syncUpdate(state, {
              packages: selected,
              global: false,
              agent,
              yes: false,
              mode: 'update',
            })
            await refreshState()
            return true
          }
          case 'remove': {
            // Check if global skills exist to offer scope choice
            const globalSkills = [...iterateSkills({ scope: 'global' })]
            let removeGlobal = false
            if (globalSkills.length > 0) {
              const scope = guard(await p.select({
                message: 'Which skills?',
                options: [
                  { label: 'Project skills', value: 'local' as const },
                  { label: 'Global skills', value: 'global' as const, hint: `${globalSkills.length} installed` },
                ],
              }))
              removeGlobal = scope === 'global'
            }
            await removeCommand(state, {
              global: removeGlobal,
              agent,
              yes: false,
            })
            await refreshState()
            break
          }
          case 'search': {
            const { interactiveSearch } = await import('./commands/search-interactive.ts')
            await interactiveSearch()
            break
          }
          case 'info':
            await statusCommand({ global: false })
            break
          case 'config':
            await configCommand()
            await refreshState()
            break
          case 'prepare-hook': {
            const added = await suggestPrepareHook(cwd)
            if (added)
              needsPrepareHook = false
            break
          }
        }
      },
    })
  },
})

runMain(main)
