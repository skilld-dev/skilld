import type { AgentType, OptimizeModel } from '../agent/index.ts'
import type { FeaturesConfig } from '../core/config.ts'
import { execSync } from 'node:child_process'
import * as p from '@clack/prompts'
import { getOAuthProviderList, loginOAuthProvider } from '../agent/clis/pi-ai.ts'
import { agents, getAvailableModels, getModelName } from '../agent/index.ts'
import { isInteractive } from '../cli/env.ts'
import { NO_MODELS_MESSAGE, OAUTH_NOTE, pickModel } from '../cli/model-picker.ts'
import { defaultFeatures, updateConfig } from '../core/config.ts'

function hasGhCli(): boolean {
  if (process.env.SKILLD_NO_GH)
    return false
  try {
    execSync('gh --version', { stdio: 'ignore' })
    return true
  }
  catch {
    return false
  }
}

export interface WizardOptions {
  /** Resolved target agent, if known */
  agent?: AgentType
  /** Show next-steps outro when done (default: true) */
  showOutro?: boolean
}

export async function runWizard(opts: WizardOptions = {}): Promise<boolean> {
  if (!isInteractive())
    return false

  const agentLabel = opts.agent ? agents[opts.agent].displayName : null
  const skillsDir = opts.agent ? agents[opts.agent].skillsDir : '.claude/skills'
  const agentLine = agentLabel
    ? `\n\x1B[90mTarget agent: ${agentLabel}\x1B[0m`
    : ''

  p.note(
    `Your AI agent reads docs from its training data - but APIs change,\n`
    + `versions drift, and patterns go stale. Skilld fixes this.\n`
    + `\n`
    + `It generates a \x1B[1mSKILL.md\x1B[0m - a markdown reference card built from\n`
    + `the \x1B[1mactual docs, issues, and release notes\x1B[0m for the exact\n`
    + `package versions in your project. Your agent reads this file\n`
    + `every session - no hallucinated APIs.\n`
    + `\n`
    + `\x1B[1mHow it works:\x1B[0m\n`
    + `  1. Fetch docs, issues, and types for your packages\n`
    + `  2. Optionally compress with an LLM into a concise cheat sheet\n`
    + `\n`
    + `\x1B[90mExample: \`skilld add vue\` creates ${skillsDir}/vue-skilld/SKILL.md\n`
    + `Your agent then knows the right APIs, gotchas, and patterns\n`
    + `for your exact version.\x1B[0m${
      agentLine}`,
    'Welcome to skilld',
  )

  const ghInstalled = hasGhCli()

  if (ghInstalled) {
    p.log.success(
      'GitHub CLI detected — will use it to pull issues and discussions.',
    )
  }
  else {
    p.log.info(
      '\x1B[90mGitHub CLI not installed — issues and discussions disabled.\n'
      + '  Install later to enable: \x1B[36mhttps://cli.github.com\x1B[0m',
    )
  }

  // Feature toggles
  const selected = await p.multiselect({
    message: 'What data sources should skills include?',
    options: [
      { label: 'Local search', value: 'search' as const, hint: 'query engine for `skilld search` across all skill docs' },
      { label: 'Release notes', value: 'releases' as const, hint: 'changelogs and migration notes per version' },
      { label: 'GitHub issues', value: 'issues' as const, hint: 'common bugs, workarounds, and solutions', disabled: !ghInstalled },
      { label: 'GitHub discussions', value: 'discussions' as const, hint: 'community Q&A and usage examples', disabled: !ghInstalled },
    ],
    initialValues: [
      ...Object.entries(defaultFeatures)
        .filter(([, v]) => v)
        .map(([k]) => k),
      ...(ghInstalled ? ['issues', 'discussions'] as const : []),
    ] as Array<keyof FeaturesConfig>,
    required: false,
  })

  if (p.isCancel(selected)) {
    p.cancel('Setup cancelled')
    return false
  }

  const features: FeaturesConfig = {
    search: selected.includes('search'),
    issues: selected.includes('issues'),
    discussions: selected.includes('discussions'),
    releases: selected.includes('releases'),
  }

  // Enhancement model - optional, independent of target agent
  p.note(
    'An LLM can optionally summarize raw docs into a focused reference\n'
    + 'highlighting best practices, gotchas, and migrations.\n'
    + '\n'
    + '\x1B[1mWithout LLM:\x1B[0m  ~2 KB skill with package metadata, types, and links\n'
    + '\x1B[1mWith LLM:\x1B[0m     ~5 KB skill with curated gotchas, patterns, and migration notes\n'
    + '\n'
    + '\x1B[1mThis is a one-time build step\x1B[0m - it generates the SKILL.md, then your\n'
    + 'coding agent reads the result every session. Can be a different model.\n'
    + '\n'
    + '\x1B[90mWorks with API keys (ANTHROPIC_API_KEY, GEMINI_API_KEY, OPENAI_API_KEY)\n'
    + 'or CLI tools (claude, gemini, codex).\x1B[0m',
    'Enhancement model (optional)',
  )

  let modelId: OptimizeModel | undefined
  let skippedEnhancement = false
  let oauthJustConnected = false

  // Loop so user can connect OAuth then come back to pick a model
  while (true) {
    const allModels = process.env.SKILLD_NO_AGENTS ? [] : await getAvailableModels()

    if (allModels.length === 0) {
      p.log.warn(NO_MODELS_MESSAGE)
    }
    else if (oauthJustConnected) {
      p.log.step(`${allModels.length} models now available. Select one below.`)
    }
    else {
      // Show which providers were found by name (e.g. "Anthropic via CLI, OpenAI via API key")
      const providers = new Set<string>()
      for (const m of allModels) {
        const vendor = m.vendorGroup ?? m.providerName
        if (!m.id.startsWith('pi:'))
          providers.add(`${vendor} via CLI`)
        else if (m.hint?.includes('API key'))
          providers.add(`${vendor} via API key`)
        else if (m.hint?.includes('OAuth'))
          providers.add(`${vendor} via OAuth`)
      }
      if (providers.size > 0)
        p.log.success(`Found: ${[...providers].join(', ')}`)
    }

    const oauthProviders = getOAuthProviderList()
    const afterOptions = oauthProviders.length > 0
      ? [
          { label: '⚠ Connect OAuth provider...', value: '_connect', hint: 'may violate provider ToS' },
          { label: 'Skip enhancement', value: '_skip', hint: 'base skill with docs, issues, and types, add LLM later via `skilld config`' },
        ]
      : [
          { label: 'Skip enhancement', value: '_skip', hint: 'base skill with docs, issues, and types, add LLM later via `skilld config`' },
        ]

    const choice = await pickModel(allModels, {
      before: allModels.length > 0
        ? [{ label: 'Auto', value: '_auto', hint: 'picks best available model from connected providers' }]
        : [],
      after: afterOptions,
    })

    if (choice === null) {
      p.cancel('Setup cancelled')
      return false
    }

    if (choice === '_connect') {
      await wizardConnectProvider()
      oauthJustConnected = true
      continue
    }

    if (choice === '_skip') {
      skippedEnhancement = true
      break
    }
    if (choice === '_auto')
      break

    modelId = choice as OptimizeModel
    break
  }

  updateConfig({
    features,
    ...(modelId
      ? { model: modelId, skipLlm: false }
      : { model: undefined, skipLlm: skippedEnhancement }),
  })

  // Summary of what was saved
  const modelSummary = modelId
    ? getModelName(modelId)
    : skippedEnhancement
      ? 'none (raw docs)'
      : 'auto'
  const featureList = Object.entries(features).filter(([, v]) => v).map(([k]) => k).join(', ') || 'none'
  p.log.success(`Model: ${modelSummary} · Features: ${featureList}`)

  if (opts.showOutro !== false) {
    p.note(
      'Run \x1B[36mskilld add <pkg>\x1B[0m to generate skills for specific packages\n'
      + 'Run \x1B[36mskilld\x1B[0m to scan your project and pick packages interactively\n'
      + 'Run \x1B[36mskilld config\x1B[0m to change settings later',
      'Setup complete',
    )
  }
  return true
}

async function wizardConnectProvider(): Promise<void> {
  p.note(OAUTH_NOTE, 'How OAuth works')

  const providers = getOAuthProviderList()
  const provider = await p.select({
    message: 'Connect provider',
    options: providers.map(pr => ({
      label: pr.name,
      value: pr.id,
      hint: pr.loggedIn ? 'connected' : undefined,
    })),
  })

  if (p.isCancel(provider))
    return

  const spinner = p.spinner()
  spinner.start('Connecting...')

  const success = await loginOAuthProvider(provider as string, {
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

  if (success) {
    const name = providers.find(pr => pr.id === provider)?.name ?? provider
    p.log.success(`Connected to ${name}`)
  }
}
