import { defineBuildConfig } from 'obuild/config'

export default defineBuildConfig({
  entries: [
    {
      type: 'bundle',
      input: [
        './src/wire.ts',
        './src/v1.ts',
        './src/constants.ts',
        './src/test-fixtures.ts',
        './src/behaviors.ts',
      ],
      outDir: './dist',
      rolldown: {
        plugins: [{
          name: 'escape-environment-pattern',
          generateBundle(_options, bundle) {
            // Nitro replaces this text inside strings. Preserve its runtime value.
            for (const output of Object.values(bundle)) {
              if (output.type === 'chunk')
                output.code = output.code.replaceAll('"import.meta.env"', '["import.", "meta.env"].join("")')
            }
          },
        }],
      },
    },
  ],
})
