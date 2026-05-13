import { defineBuildConfig } from 'obuild/config'

export default defineBuildConfig({
  entries: [
    {
      type: 'bundle',
      input: [
        './src/cli-entry.ts',
        './src/cli.ts',
        './src/prepare.ts',
        './src/retriv/worker.ts',
      ],
      rolldown: {
        external: ['@napi-rs/keyring', /@napi-rs\/keyring-/],
      },
    },
  ],
})
