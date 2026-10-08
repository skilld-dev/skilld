# Source decisions

This Skill contains original guidance for skilld CLI work.
Two upstream Skills informed the design. Their application scaffolds and scripts were not incorporated.

## tui-design

Author: Diego Marino.
Repository: `diegomarino/tui-design`.
Revision: `57c199f22dbf04f057c279431d45346cc1fd5dcd`.
[Reviewed SKILL.md](https://github.com/diegomarino/tui-design/blob/57c199f22dbf04f057c279431d45346cc1fd5dcd/skills/tui-design/SKILL.md).
License: MIT, declared in the reviewed Skill frontmatter.

Adopted ideas:

- Classify static output, pickers, and full-screen interactions separately.
- Use semantic colours with independent selection and status cues.
- Design around terminal cells and inspect actual rendered evidence.
- Preserve selection and distinguish unknown totals from measured progress.
- Treat plain output and terminal restoration as product behavior.

Excluded prescriptions:

- Mandatory concept counts, large token schemas, and subjective craft scores.
- Python mock tooling and copied starter applications.
- Forced dark themes or universal shortcut sets.
- Mandatory NDJSON, additive-only schemas, and errors-last ordering.

These prescriptions conflict with skilld's existing contracts or add work without improving the user's decision.

## tui-development

Author: NVIDIA.
Repository: `NVIDIA/OpenShell`.
Revision: `fb8f6c0885127fe4ca29993739131a3bdbaed477`.
[Reviewed SKILL.md](https://github.com/NVIDIA/OpenShell/blob/fb8f6c0885127fe4ca29993739131a3bdbaed477/.agents/skills/tui-development/SKILL.md).
License: [Apache-2.0 at the reviewed revision](https://github.com/NVIDIA/OpenShell/blob/fb8f6c0885127fe4ca29993739131a3bdbaed477/LICENSE).

Adopted ideas:

- Keep long operations outside the input loop.
- Represent lifecycle states explicitly and route background results through events.
- Make focus, valid actions, scope, and outcomes visible.
- Review file changes before applying them.

Excluded prescriptions:

- OpenShell branding, topology, deployment commands, and splash screens.
- Mandatory Tokio, unbounded channels, and fixed network timeouts.
- Independent pending flags and forced interruption during writes.

The synthesis uses skilld's current host boundaries, shared rendering, and terminal lifecycle.
It adds durable shell summaries and actual terminal journey verification beyond the upstream build guidance.
It introduces no deployment, privilege, or installation requirements.
