/**
 * Agent module - detection, installation, and skill optimization
 */

export type { CustomPrompt, ModelInfo, OptimizeDocsOptions, OptimizeModel, OptimizeResult, SkillSection, StreamProgress } from './clis/index.ts'
// CLI optimization
export {
  buildAllSectionPrompts,
  buildSectionPrompt,
  createToolProgress,
  getAvailableModels,
  getModelLabel,
  getModelName,
  SECTION_MERGE_ORDER,
  SECTION_OUTPUT_FILES,
} from './clis/index.ts'
// Import detection
export { detectImportedPackages } from './detect-imports.ts'
// Detection
export { detectEnvAgent, detectInstalledAgents, detectProjectAgents, detectTargetAgent, getAgentVersion } from './detect.ts'
// Installation
export { computeSkillDirName, installSkillForAgents, linkSkillToAgents, sanitizeName, unlinkSkillFromAgents } from './install.ts'

export { optimizeDocs } from './llm-enhancer.ts'

// Skill generation
export { extractMarkedSections, generateSkillMd, getSectionValidator, portabilizePrompt, wrapSection, writeGeneratedSkillMd, writeSkillMd } from './prompts/index.ts'

export type { SkillOptions } from './prompts/index.ts'
// Targets
export { agents } from './targets/index.ts'
export type { AgentTarget, FrontmatterField } from './targets/index.ts'

// Types
export type { AgentType, SkillMetadata } from './types.ts'
