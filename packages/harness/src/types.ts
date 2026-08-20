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

export interface SkillRunOptions {
  readonly signal?: AbortSignal
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
  /** Cleanup problems after the new Skill reached its destination. */
  readonly warnings: ReadonlyArray<string>
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

export type SkillRunResult
  = | { readonly _tag: 'Ok', readonly value: GeneratedSkill | SkillReview }
    | { readonly _tag: 'Err', readonly error: SkillRunError }

export interface SkillHarness {
  readonly run: (input: SkillRun, options?: SkillRunOptions) => Promise<SkillRunResult>
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
