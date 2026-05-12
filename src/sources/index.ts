/**
 * Doc resolution: turning a Package into Reference content.
 *
 * Two stages, per CONTEXT.md:
 *
 *   1. URL resolution — npm/crates/registry → ResolvedPackage with discovered URLs.
 *      Cascade-stateful, lives in `npm.ts` / `crates.ts` / `resolve-package.ts`.
 *
 *   2. Content resolution — ResolvedPackage → in-memory docs.
 *      Pure (no fs), lives in `content-resolver.ts`. Persistence is the caller's job.
 *
 * Everything below `// ─ Content fetching` is stage-2 input fetchers and helpers.
 */

// ─ Stage 1: URL resolution ───────────────────────────────────────────────

export { fetchBlogReleases } from './blog-releases.ts'
export { resolveCrateDocsWithAttempts } from './crates.ts'

export { fetchCrawledDocs, toCrawlPattern } from './crawl.ts'

export type { GitHubDiscussion } from './discussions.ts'
export {
  fetchGitHubDiscussions,
  formatDiscussionAsMarkdown,
  generateDiscussionIndex,
} from './discussions.ts'

export { generateDocsIndex } from './docs.ts'

export type { EntryFile } from './entries.ts'
export { resolveEntryFiles } from './entries.ts'

// ─ Stage 2: Content fetching (inputs to content-resolver) ────────────────

export type { GitSkillSource, RemoteSkill } from './git-skills.ts'
export {
  fetchGitSkills,
  parseGitSkillInput,
  parseSkillFrontmatterName,
} from './git-skills.ts'

export type { GitDocsResult } from './github.ts'

export {
  fetchGitDocs,
  fetchGitHubRepoMeta,
  fetchReadme,
  fetchReadmeContent,
  filterFrameworkDocs,
  isShallowGitDocs,
  MIN_GIT_DOCS,
  resolveGitHubRepo,
  validateGitDocsWithLlms,
} from './github.ts'

export type { GitHubIssue } from './issues.ts'

export {
  fetchGitHubIssues,
  formatIssueAsMarkdown,
  generateIssueIndex,
  isGhAvailable,
} from './issues.ts'
export {
  downloadLlmsDocs,
  extractSections,
  fetchLlmsTxt,
  fetchLlmsUrl,
  normalizeLlmsLinks,
  parseMarkdownLinks,
} from './llms.ts'

export { resolveLocalDep } from './local-dep.ts'

// ─ GitHub timeline (issues, discussions, releases) ───────────────────────

export type { LocalPackageInfo } from './local-package.ts'
export {
  getInstalledSkillVersion,
  parseVersionSpecifier,
  readLocalDependencies,
  readLocalPackageInfo,
  resolveInstalledVersion,
  resolveLocalPackageDocs,
} from './local-package.ts'

export {
  fetchLatestVersion,
  fetchNpmPackage,
  fetchNpmRegistryMeta,
  fetchPkgDist,
  searchNpmPackages,
} from './npm-registry.ts'

export type { BlogPreset, BlogRelease, DocOverride } from './package-registry.ts'
export {
  getBlogPreset,
  getCrawlUrl,
  getDocOverride,
  getFilePatterns,
  getPrereleaseChangelogRef,
  getRelatedPackages,
  getRepoEntry,
  getRepoKeyForPackage,
} from './package-registry.ts'

export type { GitHubRelease, ReleaseIndexOptions, SemVer } from './releases.ts'
export { compareSemver, fetchReleaseNotes, generateReleaseIndex, isPrerelease, parseSemver } from './releases.ts'

export type { PackageResolution, ResolvePackageOptions } from './resolve-package.ts'
export { resolvePackageOrCrate } from './resolve-package.ts'

// ─ Pre-authored skills from git repos (separate flow) ────────────────────

export type {
  ContentResolver,
  ResolveCtx,
  Resolver,
  ResolverOutcome,
} from './resolver-registry.ts'
export { createContentResolver, defineResolver } from './resolver-registry.ts'

export type { ResolveOptions, ResolveStep } from './resolver.ts'
export { resolvePackageDocs, resolvePackageDocsWithAttempts } from './resolver.ts'

export { crawlUrlResolver } from './resolvers/crawl-url.ts'
export { defaultResolvers } from './resolvers/default.ts'
export { gitTagResolver } from './resolvers/git-tag.ts'
export { githubMetaResolver } from './resolvers/github-meta.ts'
export { githubReadmeResolver } from './resolvers/github-readme.ts'
export { githubSearchResolver } from './resolvers/github-search.ts'
export { llmsTxtResolver } from './resolvers/llms-txt.ts'
export { localReadmeResolver } from './resolvers/local-readme.ts'
export { npmResolver } from './resolvers/npm.ts'

// ─ Shared types and utilities ────────────────────────────────────────────

export type {
  FetchedDoc,
  LlmsContent,
  LlmsLink,
  LocalDependency,
  NpmPackageInfo,
  ResolveAttempt,
  ResolvedPackage,
  ResolveResult,
} from './types.ts'

export {
  $fetch,
  fetchGitHubRaw,
  fetchText,
  verifyUrl,
} from './utils.ts'
