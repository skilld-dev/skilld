/**
 * Upgrade-runbook synthesis prompt.
 *
 * The primary consumer is a coding agent handed this via `npx skilld add`, so
 * the output is an *executable runbook*, not a prose guide: breaking changes
 * first (the only changes that require action), as concrete transformations the
 * agent applies top-to-bottom, then verification. It stays valid Markdown so it
 * reads fine for humans on the web too.
 *
 * Input is the pre-bucketed breaking + code-affecting feature changes; fixes and
 * improvements are passed only as counts (context, no action needed).
 */

export interface GuidePromptInput {
  packageName: string
  /** Largest version we are migrating TO (may be a prerelease). */
  version: string
  /** Stable version we are migrating FROM, when known. */
  fromVersion?: string
  prerelease: boolean
  repoUrl?: string
  /** Bucketed breaking + feature changes (Markdown sections). */
  material: string
  /** Fixes/improvements counts — surfaced as context, not steps. */
  contextCounts?: { fixes: number, improvements: number }
}

export function buildGuidePrompt(input: GuidePromptInput): string {
  const { packageName, version, fromVersion, prerelease, repoUrl, material, contextCounts } = input
  const fromClause = fromVersion ? `from \`${fromVersion}\` to \`${version}\`` : `to \`${version}\``
  const prereleaseNote = prerelease
    ? `\nThis is a prerelease (\`${version}\`) — state that in the summary and note APIs may change before stable.`
    : ''
  const contextNote = contextCounts && (contextCounts.fixes || contextCounts.improvements)
    ? `\nThis release also contains ${contextCounts.fixes} bug fixes and ${contextCounts.improvements} improvements that need no code changes — mention them in one closing line, do NOT expand them into steps.`
    : ''

  return `You are producing an EXECUTABLE UPGRADE RUNBOOK that a coding agent will follow to upgrade the npm package \`${packageName}\` ${fromClause}.${prereleaseNote}${contextNote}

The reader is an agent that will apply each step to a real codebase. Optimise for execution: imperative voice, code-first, every step concrete and verifiable. No marketing, no narrative.

BUCKETED CHANGES (breaking changes + code-affecting features, already extracted from the release notes):
<changes>
${material}
</changes>

Output ONLY GitHub-flavoured Markdown, no preamble or closing commentary, no fence around the whole document.

Structure:

# Migrating ${packageName} to ${version}

1. A 2-3 sentence summary: what this upgrade requires and who must act.
2. \`## Breaking changes\` — the mandatory work. Each as its own \`###\` subsection: a one-line statement of what changed, a \`diff\` or before/after block when the source shows the code, and a \`grep\`/find pattern to locate affected code. Omit only if there are genuinely none.
3. \`## New APIs to adopt\` — code-affecting features worth adopting (optional for the agent); one per bullet with a minimal example. Omit if none.
4. \`## Upgrade steps\` — a numbered checklist the agent runs top to bottom: start with \`npm i ${packageName}@${version}\`, then ONE step per breaking change listed above (find pattern → change to apply), then any required config edits. Derive steps ONLY from the Breaking changes section — do NOT add steps for APIs that aren't listed as breaking, and do NOT speculate about deprecations. New APIs from section 3 are optional adoptions, not upgrade steps. If a step would carry a caveat like "not deprecated in this release", omit it entirely.
5. \`## Verification\` — exact commands to confirm success (\`grep\` for removed APIs returning nothing, typecheck/build, relevant CLI checks).

Rules:
- Ground every step in the bucketed changes above. Do NOT invent changes, APIs, method names, import paths, or signatures not present in the source.
- CODE FIDELITY (a wrong transformation is worse than none): only emit a \`diff\`/code block when the source shows real code or an exact API name; otherwise describe the change in prose naming the exact symbol and point to the release notes. NEVER emit a diff whose \`-\` and \`+\` lines match or whose \`+\` is a placeholder/comment.
- Keep it tight: cut anything the agent does not need to complete the upgrade.${repoUrl ? `\n- Link to ${repoUrl} for release notes where the new signature isn't shown.` : ''}`
}
