/**
 * E2E test matrix — package specs and expected outputs.
 *
 * Each entry defines what a package should resolve to, what files get cached,
 * and what the emitted SKILL.md should contain.
 *
 * Add a row here to test a new package through the full sync pipeline.
 */

// ── Types ───────────────────────────────────────────────────────────

export type Preset = 'nuxt' | 'next' | 'vue' | 'react' | 'svelte' | 'vite' | 'astro' | 'cross-framework' | 'general'

export interface PackageSpec {
  name: string
  /** Framework/ecosystem preset grouping */
  preset: Preset

  // ── Resolution expectations ──
  /** GitHub repo URL pattern (substring match) */
  expectRepoUrl: string
  /** Package has a docs homepage URL */
  expectDocsUrl: string | null
  /** Resolution sources that should succeed */
  expectSources: {
    npm: true
    gitDocs: boolean
    llmsTxt: boolean
    readme: boolean
  }

  // ── Cache expectations ──
  /** Docs type that should be used (highest priority source that resolved) */
  expectDocsType: 'docs' | 'llms.txt' | 'readme'
  /** Files or patterns expected in ~/.skilld/references/<pkg>@<ver>/ */
  expectCacheFiles: string[]
  /** Minimum total doc files (.md + .txt) in cache */
  minCacheDocs: number

  // ── SKILL.md expectations ──
  /** Expected description pattern — either glob-based or import-based */
  expectDescriptionContains: string

  // ── Search expectations ──
  /** Query + minimum hits to verify search index works */
  searchQuery?: { query: string, minHits: number }

  // ── Shipped skills expectations ──
  /** Package ships skills/ directory — skip cache/search tests */
  expectShipped?: boolean
  /** Expected skill names inside skills/ directory */
  expectShippedSkills?: string[]
}

// ── Matrix ──────────────────────────────────────────────────────────

export const PACKAGES: PackageSpec[] = [
  // ═══════════════════════════════════════════════════════════════════
  // Nuxt preset
  // ═══════════════════════════════════════════════════════════════════

  // ── nuxt ──────────────────────────────────────────────────────────
  // Big framework, git docs with 150+ files, llms.txt also available.
  // Git docs win because they're checked first.
  {
    name: 'nuxt',
    preset: 'nuxt',
    expectRepoUrl: 'github.com/nuxt/nuxt',
    expectDocsUrl: 'https://nuxt.com',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: false },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/1.getting-started/01.introduction.md',
      'docs/1.getting-started/10.data-fetching.md',
      'docs/3.guide/1.concepts/3.auto-imports.md',
      'docs/4.api/2.composables/use-fetch.md',
    ],
    minCacheDocs: 100,
    expectDescriptionContains: '"nuxt"',
    searchQuery: { query: 'composable', minHits: 1 },
  },

  // ── pinia ─────────────────────────────────────────────────────────
  // Vue/Nuxt state management — git docs in packages/docs/ monorepo (prefix stripped to docs/).
  {
    name: 'pinia',
    preset: 'nuxt',
    expectRepoUrl: 'github.com/vuejs/pinia',
    expectDocsUrl: 'https://pinia.vuejs.org',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/core-concepts/index.md',
    ],
    minCacheDocs: 5,
    expectDescriptionContains: '"pinia"',
    searchQuery: { query: 'store', minHits: 1 },
  },

  // ── @nuxt/content ─────────────────────────────────────────────────
  // Content module — docs at content.nuxt.com, git docs in monorepo.
  // Also has llms.txt at content.nuxt.com.
  {
    name: '@nuxt/content',
    preset: 'nuxt',
    expectRepoUrl: 'github.com/nuxt/content',
    expectDocsUrl: 'https://content.nuxt.com',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/content/docs/3.files/1.markdown.md',
    ],
    minCacheDocs: 30,
    expectDescriptionContains: '"@nuxt/content"',
    searchQuery: { query: 'content', minHits: 1 },
  },

  // ═══════════════════════════════════════════════════════════════════
  // Next.js preset
  // ═══════════════════════════════════════════════════════════════════

  // ── next ──────────────────────────────────────────────────────────
  // React framework — git docs + llms.txt at nextjs.org.
  {
    name: 'next',
    preset: 'next',
    expectRepoUrl: 'github.com/vercel/next.js',
    expectDocsUrl: 'https://nextjs.org',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: false },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/01-app/01-getting-started/01-installation.mdx',
    ],
    minCacheDocs: 50,
    expectDescriptionContains: '"next"',
    searchQuery: { query: 'routing', minHits: 1 },
  },

  // ── @tanstack/react-query ─────────────────────────────────────────
  // Data fetching — git docs in monorepo docs/ folder.
  {
    name: '@tanstack/react-query',
    preset: 'next',
    expectRepoUrl: 'github.com/TanStack/query',
    expectDocsUrl: 'https://tanstack.com/query',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/framework/react/overview.md',
    ],
    minCacheDocs: 5,
    expectDescriptionContains: '"@tanstack/react-query"',
    searchQuery: { query: 'query', minHits: 1 },
  },

  // ═══════════════════════════════════════════════════════════════════
  // Vue preset
  // ═══════════════════════════════════════════════════════════════════

  // ── vue ───────────────────────────────────────────────────────────
  // Core runtime — npm name is "vue" but repo is vuejs/core.
  // No git docs/ folder in the package, but llms.txt at vuejs.org.
  // llms.txt has linked .md files → downloads into docs/.
  {
    name: 'vue',
    preset: 'vue',
    expectRepoUrl: 'github.com/vuejs/core',
    expectDocsUrl: 'https://vuejs.org/',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/guide/essentials/reactivity-fundamentals.md',
      'docs/api/reactivity-core.md',
      'docs/style-guide/rules-essential.md',
    ],
    minCacheDocs: 50,
    expectDescriptionContains: '"vue"',
    searchQuery: { query: 'reactivity', minHits: 1 },
  },

  // ── vue-router ────────────────────────────────────────────────────
  // Official router — git docs in packages/docs/ monorepo (prefix stripped to docs/).
  {
    name: 'vue-router',
    preset: 'vue',
    expectRepoUrl: 'github.com/vuejs/router',
    expectDocsUrl: 'https://router.vuejs.org',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/guide/index.md',
    ],
    minCacheDocs: 5,
    expectDescriptionContains: '"vue-router"',
    searchQuery: { query: 'route', minHits: 1 },
  },

  // ── @vueuse/core ──────────────────────────────────────────────────
  // Composition utilities — 300+ composable docs via monorepo (prefix stripped to docs/).
  {
    name: '@vueuse/core',
    preset: 'vue',
    expectRepoUrl: 'github.com/vueuse/vueuse',
    expectDocsUrl: 'https://vueuse.org',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/core/useActiveElement/index.md',
    ],
    minCacheDocs: 100,
    expectDescriptionContains: '"@vueuse/core"',
    searchQuery: { query: 'composable', minHits: 1 },
  },

  // ── motion-v ────────────────────────────────────────────────────────
  // Motion for Vue — llms.txt available at motion.dev.
  {
    name: 'motion-v',
    preset: 'vue',
    expectRepoUrl: 'github.com/motiondivision/motion-vue',
    expectDocsUrl: 'https://motion.dev',
    expectSources: { npm: true, gitDocs: false, llmsTxt: true, readme: true },
    expectDocsType: 'llms.txt',
    expectCacheFiles: [
      'llms.txt',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"motion-v"',
  },

  // ═══════════════════════════════════════════════════════════════════
  // React preset
  // ═══════════════════════════════════════════════════════════════════

  // ── react ─────────────────────────────────────────────────────────
  // Core library — no git docs folder, llms.txt at react.dev.
  // Docs come from llms.txt linked .md downloads → docs/.
  {
    name: 'react',
    preset: 'react',
    expectRepoUrl: 'github.com/react/react',
    expectDocsUrl: 'https://react.dev/',
    expectSources: { npm: true, gitDocs: false, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'llms.txt',
    ],
    minCacheDocs: 100,
    expectDescriptionContains: '"react"',
    searchQuery: { query: 'component', minHits: 1 },
  },

  // ── zustand ───────────────────────────────────────────────────────
  // Minimal state management — git docs in docs/ folder.
  {
    name: 'zustand',
    preset: 'react',
    expectRepoUrl: 'github.com/pmndrs/zustand',
    expectDocsUrl: 'https://zustand-demo.pmnd.rs/',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/learn/guides/updating-state.md',
    ],
    minCacheDocs: 3,
    expectDescriptionContains: '"zustand"',
  },

  // ── react-hook-form ───────────────────────────────────────────────
  // Form library — git docs has translated READMEs in docs/ folder.
  {
    name: 'react-hook-form',
    preset: 'react',
    expectRepoUrl: 'github.com/react-hook-form/react-hook-form',
    expectDocsUrl: 'https://react-hook-form.com',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/Template.md',
    ],
    minCacheDocs: 10,
    expectDescriptionContains: '"react-hook-form"',
  },

  // ═══════════════════════════════════════════════════════════════════
  // Svelte preset
  // ═══════════════════════════════════════════════════════════════════

  // ── svelte ────────────────────────────────────────────────────────
  // Compiler framework — git docs + llms.txt at svelte.dev.
  {
    name: 'svelte',
    preset: 'svelte',
    expectRepoUrl: 'github.com/sveltejs/svelte',
    expectDocsUrl: 'https://svelte.dev',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/01-introduction/01-overview.md',
    ],
    minCacheDocs: 10,
    expectDescriptionContains: '"svelte"',
    searchQuery: { query: 'component', minHits: 1 },
  },

  // ── @sveltejs/kit ─────────────────────────────────────────────────
  // SvelteKit framework — docs moved to sveltejs/svelte repo. Git docs in kit
  // are shallow (test fixtures, type stubs), falls through to llms.txt at svelte.dev.
  // llms.txt links download into docs/, so docsType becomes 'docs'.
  {
    name: '@sveltejs/kit',
    preset: 'svelte',
    expectRepoUrl: 'github.com/sveltejs/kit',
    expectDocsUrl: 'https://svelte.dev',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'llms.txt',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"@sveltejs/kit"',
  },

  // ═══════════════════════════════════════════════════════════════════
  // Vite preset
  // ═══════════════════════════════════════════════════════════════════

  // ── vite ──────────────────────────────────────────────────────────
  // Build tool — has both git docs and llms.txt. Git docs win.
  {
    name: 'vite',
    preset: 'vite',
    expectRepoUrl: 'github.com/vitejs/vite',
    expectDocsUrl: 'https://vite.dev',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/config/shared-options.md',
      'docs/guide/features.md',
      'docs/guide/api-plugin.md',
      'docs/guide/ssr.md',
    ],
    minCacheDocs: 30,
    expectDescriptionContains: '"vite"',
    searchQuery: { query: 'plugin', minHits: 1 },
  },

  // ═══════════════════════════════════════════════════════════════════
  // Astro preset
  // ═══════════════════════════════════════════════════════════════════

  // ── astro ─────────────────────────────────────────────────────────
  // Content-focused framework — docs live in withastro/docs repo (override).
  {
    name: 'astro',
    preset: 'astro',
    expectRepoUrl: 'github.com/withastro/astro',
    expectDocsUrl: 'https://astro.build',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/basics/astro-components.mdx',
    ],
    minCacheDocs: 100,
    expectDescriptionContains: '"astro"',
    searchQuery: { query: 'component', minHits: 1 },
  },

  // ═══════════════════════════════════════════════════════════════════
  // Cross-framework preset
  // ═══════════════════════════════════════════════════════════════════

  // ── autoprefixer ────────────────────────────────────────────────────
  // npm homepage is a Twitter URL — should be filtered out by isUselessDocsUrl.
  // Falls through to GitHub meta for docsUrl. No git docs folder.
  {
    name: 'autoprefixer',
    preset: 'cross-framework',
    expectRepoUrl: 'github.com/postcss/autoprefixer',
    expectDocsUrl: null,
    expectSources: { npm: true, gitDocs: false, llmsTxt: false, readme: true },
    expectDocsType: 'readme',
    expectCacheFiles: [
      'docs/README.md',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"autoprefixer"',
  },

  // ── tailwindcss ───────────────────────────────────────────────────
  // CSS framework — docs live in tailwindlabs/tailwindcss.com repo (override).
  {
    name: 'tailwindcss',
    preset: 'cross-framework',
    expectRepoUrl: 'github.com/tailwindlabs/tailwindcss',
    expectDocsUrl: 'https://tailwindcss.com',
    expectSources: { npm: true, gitDocs: true, llmsTxt: false, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/accent-color.mdx',
    ],
    minCacheDocs: 100,
    expectDescriptionContains: '"tailwindcss"',
    searchQuery: { query: 'flex', minHits: 1 },
  },

  // ── drizzle-orm ───────────────────────────────────────────────────
  // TypeScript ORM — git docs too shallow (<5 files), falls through to llms.txt.
  {
    name: 'drizzle-orm',
    preset: 'cross-framework',
    expectRepoUrl: 'github.com/drizzle-team/drizzle-orm',
    expectDocsUrl: 'https://orm.drizzle.team',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'llms.txt',
    expectCacheFiles: [
      'llms.txt',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"drizzle-orm"',
  },

  // ── @trpc/server ──────────────────────────────────────────────────
  // Type-safe API layer — git docs in www/docs/.
  {
    name: '@trpc/server',
    preset: 'cross-framework',
    expectRepoUrl: 'github.com/trpc/trpc',
    expectDocsUrl: 'https://trpc.io',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/skills/trpc-router/SKILL.md',
    ],
    minCacheDocs: 15,
    expectDescriptionContains: '"@trpc/server"',
    searchQuery: { query: 'router', minHits: 1 },
  },

  // ═══════════════════════════════════════════════════════════════════
  // General preset
  // ═══════════════════════════════════════════════════════════════════

  // ── zod ───────────────────────────────────────────────────────────
  // Schema library — git docs discovered in packages/docs/content/ (monorepo).
  // Also has llms.txt at zod.dev. Git docs win because checked first.
  {
    name: 'zod',
    preset: 'general',
    expectRepoUrl: 'github.com/colinhacks/zod',
    expectDocsUrl: 'https://zod.dev',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/content/basics.mdx',
      'docs/content/api.mdx',
    ],
    minCacheDocs: 10,
    expectDescriptionContains: '"zod"',
  },

  // ── @clack/prompts ────────────────────────────────────────────────
  // CLI prompts library — no real llms.txt (bomb.sh returns HTML). README only.
  {
    name: '@clack/prompts',
    preset: 'general',
    expectRepoUrl: 'github.com/bombshell-dev/clack',
    expectDocsUrl: 'https://bomb.sh/docs/clack/basics/getting-started/',
    expectSources: { npm: true, gitDocs: false, llmsTxt: false, readme: true },
    expectDocsType: 'readme',
    expectCacheFiles: [
      'docs/README.md',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"@clack/prompts"',
  },

  // ── citty ─────────────────────────────────────────────────────────
  // Tiny CLI framework — no docs URL, no llms.txt. README only.
  {
    name: 'citty',
    preset: 'general',
    expectRepoUrl: 'github.com/unjs/citty',
    expectDocsUrl: null,
    expectSources: { npm: true, gitDocs: false, llmsTxt: false, readme: true },
    expectDocsType: 'readme',
    expectCacheFiles: [
      'docs/README.md',
    ],
    minCacheDocs: 1,
    expectDescriptionContains: '"citty"',
  },

  // ── mdream ────────────────────────────────────────────────────────
  // Small utility — README only.
  {
    name: 'mdream',
    preset: 'general',
    expectRepoUrl: 'github.com/harlan-zw/mdream',
    expectDocsUrl: 'https://mdream.dev',
    expectSources: { npm: true, gitDocs: false, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/llms-txt/guide.md',
    ],
    minCacheDocs: 10,
    expectDescriptionContains: '"mdream"',
  },

  // ── @slidev/cli ────────────────────────────────────────────────────
  // Ships its own skills/ directory in the npm package (55 files).
  // Also has git docs and readme — llms.txt no longer available at sli.dev.
  // Shipped skills take priority — no cache, no generated SKILL.md, no search.db.
  {
    name: '@slidev/cli',
    preset: 'general',
    expectRepoUrl: 'github.com/slidevjs/slidev',
    expectDocsUrl: 'https://sli.dev',
    expectSources: { npm: true, gitDocs: true, llmsTxt: true, readme: true },
    expectDocsType: 'docs',
    expectCacheFiles: [
      'docs/guide/index.md',
    ],
    minCacheDocs: 10,
    expectDescriptionContains: '"@slidev/cli"',
    searchQuery: { query: 'slide', minHits: 1 },
  },
]
