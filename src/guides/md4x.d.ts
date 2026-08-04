// md4x ships types only for its `./wasm` subpath; the root export (napi on
// node) is untyped. Declare just the surface we use.
declare module 'md4x' {
  interface Md4xAst {
    nodes: unknown[]
    frontmatter: Record<string, unknown>
    meta: Record<string, unknown>
  }
  export function parseAST(markdown: string): Promise<Md4xAst>
}
