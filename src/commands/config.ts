import type { OptimizeModel } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import { styleText } from 'node:util'
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
import { DEFAULT_EMBED_DEVICE, DEFAULT_EMBED_MODEL, EMBED_DEVICES, EMBED_MODELS, resolveEmbedModel } from '../retriv/models.ts'
import { getAvailableOllamaEmbedModels, isOllamaEmbedModel } from '../retriv/ollama-embeddings.ts'

export async function configCommand(): Promise<void> {
  const initConfig = readConfig()
  const agentId = initConfig.agent || detectTargetAgent() || undefined
  const cyan = (s: string) => styleText('cyan', s)
  const modelLabel = initConfig.skipLlm
    ? 'skip'
    : initConfig.model
      ? cyan(getModelName(initConfig.model))
      : 'auto'
  const agentLabel = agentId && agents[agentId as keyof typeof agents]
    ? cyan(agents[agentId as keyof typeof agents].displayName)
    : 'auto-detect'
  p.note(styleText('gray', `Fetch docs → Enhance with ${modelLabel} → Install to ${agentLabel}`), 'How skilld works')

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
      const embedModel = resolveEmbedModel(config.embedModel)
      const embedHint = features.search
        ? `${embedModel} · local model powering skilld search`
        : `${embedModel} · search is disabled in Data sources`
      const embedDevice = config.embedDevice || DEFAULT_EMBED_DEVICE
      options.push(
        { label: 'Enhancement model', value: 'model', hint: `${modelHint} · rewrites SKILL.md with best practices` },
        { label: 'Embedding model', value: 'embedModel', hint: embedHint },
        { label: 'Embedding device', value: 'embedDevice', hint: `${embedDevice} · where the embedding model runs` },
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

        case 'embedModel': {
          await configureEmbedModel()
          break
        }

        case 'embedDevice': {
          await configureEmbedDevice()
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
        hint: pr.loggedIn ? styleText('green', 'connected') : 'not connected',
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
          await logoutOAuthProvider(providerId as string)
          p.log.success(`Disconnected from ${pr.name}`)
        }
        return
      }

      const spinner = p.spinner()
      spinner.start('Connecting...')

      const success = await loginOAuthProvider(providerId as string, {
        onAuth: (url, instructions) => {
          spinner.stop('Open this URL in your browser:')
          p.log.info(`  ${styleText('cyan', url)}`)
          if (instructions)
            p.log.info(`  ${styleText('gray', instructions)}`)
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

// ── Embedding model selection ────────────────────────────────────────

async function configureEmbedModel(): Promise<void> {
  const config = readConfig()
  const current = resolveEmbedModel(config.embedModel)
  const envOverride = process.env.SKILLD_EMBED_MODEL?.trim()

  if (envOverride) {
    p.log.warn(`SKILLD_EMBED_MODEL is set to ${envOverride} and overrides this setting for the current shell.`)
  }

  const builtIn = EMBED_MODELS.map(m => ({
    label: m.label,
    value: m.id,
    hint: `${m.dimensions}d · ${m.hint}`,
  }))
  // Locally-pulled Ollama models are additive: an unreachable daemon simply
  // contributes nothing rather than blocking the picker.
  const ollama = (await getAvailableOllamaEmbedModels()).map(m => ({
    label: m.name,
    value: m.id,
    hint: m.hint,
  }))

  const choice = guard(await p.select({
    message: 'Embedding model: indexes and queries docs for skilld search',
    options: [...builtIn, ...ollama],
    initialValue: current,
  }))

  if (choice === config.embedModel || (choice === DEFAULT_EMBED_MODEL && !config.embedModel)) {
    p.log.info(`Embedding model unchanged (${choice})`)
    return
  }

  updateConfig({ embedModel: choice === DEFAULT_EMBED_MODEL ? undefined : choice as string })
  p.log.success(`Embedding model set to ${choice}`)
  p.log.warn('Run `skilld update` to rebuild existing search indexes with this model.')
}

// ── Embedding device selection ───────────────────────────────────────

async function configureEmbedDevice(): Promise<void> {
  const config = readConfig()
  const current = config.embedDevice || DEFAULT_EMBED_DEVICE

  const isOllama = isOllamaEmbedModel(resolveEmbedModel(config.embedModel))
  if (isOllama) {
    p.log.warn('The active embedding model runs inside Ollama, which manages its own device. This setting will have no effect until you switch to a built-in model.')
  }
  const envOverride = process.env.SKILLD_EMBED_DEVICE?.trim()

  if (envOverride)
    p.log.warn(`SKILLD_EMBED_DEVICE is set to ${envOverride} and overrides this setting for the current shell.`)

  p.note(
    'The fastest backend depends on your hardware. On an Apple M5 Max, WebGPU\n'
    + 'ran 2.6-2.9x faster than CPU across every model size, while CoreML ran\n'
    + '3-8x slower. Benchmark before trusting a device on other machines.',
    'Choosing a device',
  )

  const choice = guard(await p.select({
    message: 'Embedding device: where the model runs',
    options: EMBED_DEVICES.map(d => ({ label: d.label, value: d.id, hint: d.hint })),
    initialValue: current,
  }))

  updateConfig({ embedDevice: choice === DEFAULT_EMBED_DEVICE ? undefined : choice as string })
  p.log.success(`Embedding device set to ${choice}`)

  if (!isOllama)
    p.log.warn('Run `skilld update` to rebuild existing search indexes on this device.')

  if (choice !== DEFAULT_EMBED_DEVICE && choice !== 'cpu') {
    p.log.info('If indexing fails to start, the backend is unavailable on this machine. Switch back to Auto.')
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
