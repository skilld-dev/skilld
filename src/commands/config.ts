import type { OptimizeModel } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import * as p from '@clack/prompts'
import { defineCommand } from 'citty'
import { getOAuthProviderList, loginOAuthProvider, logoutOAuthProvider } from '../agent/clis/pi-ai.ts'
import { agents, detectTargetAgent, getAvailableModels, getModelName } from '../agent/index.ts'
import { requireInteractive } from '../cli/env.ts'
import { introLine } from '../cli/intro.ts'
import { guard, menuLoop } from '../cli/menu.ts'
import { NO_MODELS_MESSAGE, OAUTH_NOTE, pickModel } from '../cli/model-picker.ts'
import { defaultFeatures, readConfig, updateConfig } from '../core/config.ts'
import { getProjectState } from '../core/skills.ts'

export async function configCommand(): Promise<void> {
  const initConfig = readConfig()
  const agentId = initConfig.agent || detectTargetAgent() || undefined
  const cyan = (s: string) => `\x1B[36m${s}\x1B[90m`
  const modelLabel = initConfig.skipLlm
    ? 'skip'
    : initConfig.model
      ? cyan(getModelName(initConfig.model))
      : 'auto'
  const agentLabel = agentId && agents[agentId as keyof typeof agents]
    ? cyan(agents[agentId as keyof typeof agents].displayName)
    : 'auto-detect'
  p.note(`\x1B[90mFetch docs → Enhance with ${modelLabel} → Install to ${agentLabel}\x1B[0m`, 'How skilld works')

  await menuLoop({
    message: 'Settings',
    options: () => {
      const config = readConfig()
      const features = config.features ?? defaultFeatures
      const enabledCount = Object.values(features).filter(Boolean).length
      const modelHint = config.skipLlm
        ? 'disabled'
        : config.model
          ? getModelName(config.model)
          : 'auto'
      const oauthProviders = getOAuthProviderList()
      const options = [
        { label: 'Data sources', value: 'features', hint: `${enabledCount}/4 enabled · issues, releases, search, discussions` },
      ]
      if (oauthProviders.length > 0) {
        const connectedOAuth = oauthProviders.filter(pr => pr.loggedIn).length
        const oauthHint = connectedOAuth > 0 ? `${connectedOAuth} connected` : 'none'
        options.push({ label: 'OAuth providers', value: 'oauth', hint: `${oauthHint} · ⚠ may violate provider ToS` })
      }
      options.push(
        { label: 'Enhancement model', value: 'model', hint: `${modelHint} · rewrites SKILL.md with best practices` },
        { label: 'Target agent', value: 'agent', hint: `${config.agent || 'auto-detect'} · where skills are installed` },
      )
      return options
    },
    onSelect: async (action) => {
      switch (action) {
        case 'features': {
          const config = readConfig()
          const features = config.features ?? defaultFeatures
          const selected = guard(await p.multiselect({
            message: 'Data sources',
            options: [
              { label: 'Semantic + token search', value: 'search' as const, hint: 'local query engine to cut token costs and speed up grep' },
              { label: 'Release notes', value: 'releases' as const, hint: 'track changelogs for installed packages' },
              { label: 'GitHub issues', value: 'issues' as const, hint: 'surface common problems and solutions' },
              { label: 'GitHub discussions', value: 'discussions' as const, hint: 'include Q&A and community knowledge' },
            ],
            initialValues: Object.entries(features)
              .filter(([, v]) => v)
              .map(([k]) => k) as Array<keyof FeaturesConfig>,
            required: false,
          }))
          updateConfig({
            features: {
              search: selected.includes('search'),
              issues: selected.includes('issues'),
              discussions: selected.includes('discussions'),
              releases: selected.includes('releases'),
            },
          })
          p.log.success(`Data sources updated: ${selected.length} enabled`)
          break
        }

        case 'oauth': {
          await configureOAuth()
          break
        }

        case 'model': {
          await configureModel()
          break
        }

        case 'agent': {
          const config = readConfig()
          const agentChoice = guard(await p.select({
            message: 'Target agent — where should skills be installed?',
            options: [
              { label: 'Auto-detect', value: '' },
              ...Object.entries(agents).map(([id, a]) => ({
                label: a.displayName,
                value: id,
                hint: a.skillsDir,
              })),
            ],
            initialValue: config.agent || '',
          }))
          updateConfig({ agent: agentChoice || undefined })
          p.log.success(agentChoice ? `Target agent set to ${agentChoice}` : 'Target agent will be auto-detected')
          break
        }
      }
    },
  })
}

async function configureOAuth(): Promise<void> {
  p.note(OAUTH_NOTE, 'How OAuth works')

  await menuLoop({
    message: 'OAuth providers',
    options: () => {
      const providers = getOAuthProviderList()
      return providers.map(pr => ({
        label: pr.name,
        value: pr.id,
        hint: pr.loggedIn ? '\x1B[32mconnected\x1B[0m' : 'not connected',
      }))
    },
    onSelect: async (providerId) => {
      const providers = getOAuthProviderList()
      const pr = providers.find(p2 => p2.id === providerId)
      if (!pr)
        return

      if (pr.loggedIn) {
        const action = guard(await p.select({
          message: pr.name,
          options: [
            { label: 'Disconnect', value: 'disconnect' },
            { label: 'Back', value: 'back' },
          ],
        }))
        if (action === 'disconnect') {
          logoutOAuthProvider(providerId as string)
          p.log.success(`Disconnected from ${pr.name}`)
        }
        return
      }

      const spinner = p.spinner()
      spinner.start('Connecting...')

      const success = await loginOAuthProvider(providerId as string, {
        onAuth: (url, instructions) => {
          spinner.stop('Open this URL in your browser:')
          p.log.info(`  \x1B[36m${url}\x1B[0m`)
          if (instructions)
            p.log.info(`  \x1B[90m${instructions}\x1B[0m`)
          spinner.start('Waiting for authentication...')
        },
        onPrompt: async (message, placeholder) => {
          const value = await p.text({ message, placeholder })
          if (p.isCancel(value))
            return ''
          return value as string
        },
        onProgress: msg => p.log.step(msg),
      }).catch((err: Error) => {
        spinner.stop(`Login failed: ${err.message}`)
        return false
      })

      spinner.stop()
      if (success)
        p.log.success(`Connected to ${pr.name}`)
    },
  })
}

// ── Model selection ──────────────────────────────────────────────────

async function configureModel(): Promise<void> {
  // Loop so user can connect OAuth and come back to pick a model
  while (true) {
    const available = await getAvailableModels()

    if (available.length === 0)
      p.log.warn(NO_MODELS_MESSAGE)

    const oauthProviders = getOAuthProviderList()
    const afterOptions = oauthProviders.length > 0
      ? [
          { label: '⚠ Connect OAuth provider...', value: '_connect', hint: 'may violate provider ToS' },
          { label: 'Skip enhancement', value: '_skip', hint: 'base skill with docs, issues, and types' },
        ]
      : [
          { label: 'Skip enhancement', value: '_skip', hint: 'base skill with docs, issues, and types' },
        ]

    const choice = await pickModel(available, {
      before: available.length > 0
        ? [{ label: 'Auto', value: '_auto', hint: 'picks best available model from connected providers' }]
        : [],
      after: afterOptions,
    })

    if (!choice)
      return

    if (choice === '_connect') {
      await configureOAuth()
      continue
    }

    if (choice === '_skip') {
      updateConfig({ model: undefined, skipLlm: true })
      p.log.success('Enhancement disabled - skills will use raw docs only')
    }
    else if (choice === '_auto') {
      updateConfig({ model: undefined, skipLlm: false })
      p.log.success('Enhancement model will be auto-selected')
    }
    else {
      updateConfig({ model: choice as OptimizeModel, skipLlm: false })
      p.log.success(`Enhancement model set to ${getModelName(choice as OptimizeModel)}`)
    }
    return
  }
}

export const configCommandDef = defineCommand({
  meta: { name: 'config', description: 'Edit settings' },
  args: {},
  async run() {
    requireInteractive('config')
    const cwd = process.cwd()
    const state = await getProjectState(cwd)
    p.intro(introLine({ state }))
    return configCommand()
  },
})
