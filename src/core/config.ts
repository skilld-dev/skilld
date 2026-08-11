import type { OptimizeModel } from '../agent/index.ts'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { CACHE_DIR, CONFIG_PATH } from './paths.ts'
import { yamlEscape, yamlParseKV, yamlUnescape } from './yaml.ts'

const STATIC_REGEX_1 = /^ {2}(\w+):\s*(.+)/

export interface FeaturesConfig {
  search: boolean
  issues: boolean
  discussions: boolean
  releases: boolean
}

export const defaultFeatures: FeaturesConfig = {
  search: true,
  issues: true,
  discussions: true,
  releases: true,
}

/**
 * Resolve the active feature set: defaults overlaid with user config, then
 * caller-supplied overrides. Single seam so feature gating doesn't drift.
 */
export function getActiveFeatures(overrides?: Partial<FeaturesConfig>): FeaturesConfig {
  const fromConfig = readConfig().features
  const merged: FeaturesConfig = { ...defaultFeatures, ...(fromConfig ?? {}) }
  return overrides ? { ...merged, ...overrides } : merged
}

export interface SkilldConfig {
  model?: OptimizeModel
  agent?: string
  /** Local embedding model used to build and query the search index */
  embedModel?: string
  /** Execution device for the embedding model (auto, cpu, webgpu, coreml) */
  embedDevice?: string
  features?: FeaturesConfig
  projects?: string[]
  skipLlm?: boolean
}

let configCache: SkilldConfig | undefined

export function hasConfig(): boolean {
  return existsSync(CONFIG_PATH)
}

/** Whether the first-run wizard has been completed (not just agent selection) */
export function hasCompletedWizard(): boolean {
  if (!existsSync(CONFIG_PATH))
    return false
  const config = readConfig()
  return config.features !== undefined || config.model !== undefined || config.skipLlm !== undefined
}

export function readConfig(): SkilldConfig {
  if (configCache) {
    return {
      ...configCache,
      features: configCache.features ? { ...configCache.features } : undefined,
      projects: configCache.projects ? [...configCache.projects] : undefined,
    }
  }
  if (!existsSync(CONFIG_PATH))
    return {}

  const content = readFileSync(CONFIG_PATH, 'utf-8')
  const config: SkilldConfig = {}
  let inBlock: 'projects' | 'features' | null = null
  const projects: string[] = []
  const features: Partial<FeaturesConfig> = {}

  for (const line of content.split('\n')) {
    if (line.startsWith('projects:')) {
      inBlock = 'projects'
      continue
    }
    if (line.startsWith('features:')) {
      inBlock = 'features'
      continue
    }
    if (inBlock === 'projects') {
      if (line.startsWith('  - ')) {
        projects.push(yamlUnescape(line.slice(4)))
        continue
      }
      inBlock = null
    }
    if (inBlock === 'features') {
      const m = line.match(STATIC_REGEX_1)
      if (m) {
        const key = m[1] as keyof FeaturesConfig
        if (key in defaultFeatures)
          features[key] = m[2] === 'true'
        continue
      }
      inBlock = null
    }
    const kv = yamlParseKV(line)
    if (!kv)
      continue
    const [key, value] = kv
    if (key === 'model' && value)
      config.model = value as OptimizeModel
    if (key === 'agent' && value)
      config.agent = value
    if (key === 'embedModel' && value)
      config.embedModel = value
    if (key === 'embedDevice' && value)
      config.embedDevice = value
    if (key === 'skipLlm')
      config.skipLlm = value === 'true'
  }

  if (projects.length > 0)
    config.projects = projects
  if (Object.keys(features).length > 0)
    config.features = { ...defaultFeatures, ...features }
  configCache = config
  return config
}

export function writeConfig(config: SkilldConfig): void {
  mkdirSync(CACHE_DIR, { recursive: true, mode: 0o700 })

  let yaml = ''
  if (config.model)
    yaml += `model: ${config.model}\n`
  if (config.agent)
    yaml += `agent: ${config.agent}\n`
  if (config.embedModel)
    yaml += `embedModel: ${config.embedModel}\n`
  if (config.embedDevice)
    yaml += `embedDevice: ${config.embedDevice}\n`
  if (config.skipLlm)
    yaml += `skipLlm: true\n`
  if (config.features) {
    yaml += 'features:\n'
    for (const [k, v] of Object.entries(config.features)) {
      yaml += `  ${k}: ${v}\n`
    }
  }
  if (config.projects?.length) {
    yaml += 'projects:\n'
    for (const p of config.projects) {
      yaml += `  - ${yamlEscape(p)}\n`
    }
  }

  writeFileSync(CONFIG_PATH, yaml, { mode: 0o600 })
  configCache = undefined
}

export function updateConfig(updates: Partial<SkilldConfig>): void {
  const config = readConfig()
  writeConfig({ ...config, ...updates })
}

export function registerProject(projectPath: string): void {
  const config = readConfig()
  const projects = new Set(config.projects || [])
  projects.add(projectPath)
  writeConfig({ ...config, projects: [...projects] })
}

export function unregisterProject(projectPath: string): void {
  const config = readConfig()
  const projects = (config.projects || []).filter(p => p !== projectPath)
  writeConfig({ ...config, projects })
}

export function getRegisteredProjects(): string[] {
  return readConfig().projects || []
}
