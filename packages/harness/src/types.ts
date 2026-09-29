import type { HarnessV1, HarnessV1SandboxProvider } from '@ai-sdk/harness'
import type { HarnessAgentSandboxConfig } from '@ai-sdk/harness/agent'

export interface SkillDestination {
  readonly rootDir: string
  readonly name: string
}

export type PackageSource
  = | {
    readonly _tag: 'NpmPackage'
    readonly spec: string
  }
  | {
    readonly _tag: 'LocalPackage'
    readonly rootDir: string
    readonly packageDir: string
  }

export type SkillRun
  = | {
    readonly _tag: 'PackageSkill'
    readonly source: PackageSource
    readonly destination: SkillDestination
  }
  | {
    readonly _tag: 'ProjectSkill'
    readonly projectDir: string
    readonly destination: SkillDestination
  }
  | {
    readonly _tag: 'ReviewSkill'
    readonly skillDir: string
  }

export interface SkillOutputPolicy {
  readonly maxSourceFiles: number
  readonly maxSourceFileBytes: number
  readonly maxSourceBytes: number
  readonly maxOutputFiles: number
  readonly maxOutputFileBytes: number
  readonly maxOutputBytes: number
}

/** Token counts the Agent reported. A count is undefined when the Harness adapter does not report it. */
export interface SkillRunUsage {
  /** Every input token, cached ones included. */
  readonly inputTokens: number | undefined
  /** Input tokens read from the provider cache. */
  readonly cachedInputTokens: number | undefined
  readonly outputTokens: number | undefined
}

/** Progress of the Agent during a Skill run. A step is one model call. Steps count from 0. */
export type SkillRunEvent
  = | { readonly _tag: 'StepStart', readonly step: number }
    | {
      readonly _tag: 'ToolCall'
      readonly step: number
      readonly toolName: string
      readonly toolCallId: string
      readonly input: unknown
    }
    | {
      readonly _tag: 'StepFinish'
      readonly step: number
      readonly finishReason: string
      readonly usage: SkillRunUsage
    }

export interface SkillRunOptions {
  readonly signal?: AbortSignal
  /**
   * Receives progress events while the Agent works. The run does not wait for a returned promise.
   * If it throws or its promise rejects, the run continues and the result carries a warning.
   */
  readonly onEvent?: (event: SkillRunEvent) => void
}

export interface SkillFile {
  readonly path: string
  readonly bytes: number
}

export interface SourceAttempt {
  readonly source: string
  readonly status: 'used' | 'skipped'
  readonly reason?: string
}

export interface GeneratedSkill {
  readonly _tag: 'GeneratedSkill'
  readonly name: string
  readonly outputDir: string
  readonly files: ReadonlyArray<SkillFile>
  readonly sourceAttempts: ReadonlyArray<SourceAttempt>
  /**
   * Source files the Harness left out for size, and cleanup problems after
   * the new Skill reached its destination.
   */
  readonly warnings: ReadonlyArray<string>
  readonly usage: SkillRunUsage
  /** Model calls the Agent made. */
  readonly steps: number
}

export interface SkillReviewFinding {
  readonly level: 'error' | 'warning' | 'note'
  readonly path: string
  readonly message: string
  readonly fix: string
}

export interface SkillReview {
  readonly _tag: 'SkillReview'
  readonly summary: string
  readonly findings: ReadonlyArray<SkillReviewFinding>
  /** Files of the reviewed Skill that the Harness left out for size. */
  readonly warnings: ReadonlyArray<string>
  readonly usage: SkillRunUsage
  /** Model calls the Agent made. */
  readonly steps: number
}

export type SkillRunError
  = | { readonly _tag: 'InvalidInput', readonly message: string }
    | {
      readonly _tag: 'SourceUnavailable'
      readonly message: string
      readonly attempts: ReadonlyArray<SourceAttempt>
      readonly cause?: unknown
    }
    | { readonly _tag: 'AgentFailed', readonly message: string, readonly cause?: unknown }
    | { readonly _tag: 'InvalidSkill', readonly message: string, readonly issues: ReadonlyArray<string> }
    | { readonly _tag: 'UnsafeOutputPath', readonly message: string, readonly path: string }
    | { readonly _tag: 'OutputBusy', readonly message: string, readonly path: string }
    | { readonly _tag: 'PromotionFailed', readonly message: string, readonly path: string, readonly cause?: unknown }
    | { readonly _tag: 'Cancelled', readonly message: string }

/** The value each Skill run input returns. */
export interface SkillRunValues {
  readonly PackageSkill: GeneratedSkill
  readonly ProjectSkill: GeneratedSkill
  readonly ReviewSkill: SkillReview
}

export type SkillRunResult<Tag extends SkillRun['_tag'] = SkillRun['_tag']>
  = | { readonly _tag: 'Ok', readonly value: SkillRunValues[Tag] }
    | { readonly _tag: 'Err', readonly error: SkillRunError }

export interface SkillHarness {
  readonly run: <Run extends SkillRun>(input: Run, options?: SkillRunOptions) => Promise<SkillRunResult<Run['_tag']>>
}

export interface CreateSkillHarnessOptions {
  readonly harness: HarnessV1
  /** The sandbox must provide POSIX sh, rm, mkdir, and GNU find. */
  readonly sandbox: HarnessV1SandboxProvider
  /** onSession runs after the Harness writes its visible inputs. */
  readonly sandboxConfig?: HarnessAgentSandboxConfig
  readonly outputPolicy?: Partial<SkillOutputPolicy>
  /** HTTP adapter for npm metadata and immutable source archives. */
  readonly fetch?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>
}
