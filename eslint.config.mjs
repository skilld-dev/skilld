import antfu from '@antfu/eslint-config'

export default antfu({
  type: 'lib',
  rules: {
    'no-use-before-define': 'off',
    'node/prefer-global/process': 'off',
    'node/prefer-global/buffer': 'off',
    'ts/explicit-function-return-type': 'off',
    'e18e/prefer-static-regex': 'warn',
    'e18e/prefer-array-to-sorted': 'off',
  },
  ignores: [
    'CLAUDE.md',
    'docs/**',
    '.claude/skills/**',
    '.claude/worktrees/**',
    'tests/fixtures/**',
  ],
}, {
  files: ['**/*.md/**'],
  rules: {
    'style/max-statements-per-line': 'off',
  },
}, {
  files: ['**/test/**/*.ts', '**/test/**/*.js', '**/tests/**/*.mjs'],
  rules: {
    'ts/no-unsafe-function-type': 'off',
    'no-console': 'off',
    'e18e/prefer-static-regex': 'off',
  },
})
