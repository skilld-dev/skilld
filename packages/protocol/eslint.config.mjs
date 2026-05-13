import antfu from '@antfu/eslint-config'

export default antfu({
  type: 'lib',
  typescript: true,
  stylistic: true,
  ignores: ['dist', 'node_modules'],
})
