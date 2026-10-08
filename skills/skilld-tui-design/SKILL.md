---
name: skilld-tui-design
description: Designs and verifies skilld CLI output, terminal pickers, and full-screen flows. Use for terminal colours, loading feedback, keyboard controls, layout, errors, or end-to-end CLI UX reviews.
license: MIT
---

# Design skilld CLI interactions

Build the user's whole command journey, including failure, cancellation, and the return to their shell.
Read [source decisions](references/source-review.md) for the two reviewed Skills and their exact revisions.

## Establish the contract

Read the project's `GLOSSARY.md`, `COPY.md`, and `VISION.md` when present.
Keep canonical nouns and strings. Use short sentences with one idea each.
Inspect the command parser, output renderer, and terminal lifecycle before changing behavior.
Classify each affected surface as static output, a picker, a prompt, or a full-screen view.
Include command help, parser errors, startup notices, and progress indicators in the inventory.
List its starting state, successful outcome, expected failures, and cancellation behavior.
Keep the existing JSON envelope, streams, and exit codes unless the task changes that contract.

Use the existing shared renderer and semantic palette.
Avoid adding a second framework or per-screen theme.
Start with `crates/skilld-ui`, `crates/skilld-command/src/output.rs`, and `crates/skilld-native/src`.
Inspect command help and routing in `crates/skilld-command/src/lib.rs`.

## Put the decision first

Static output starts with the requested result or the failed action.
Group repeated records consistently. Place supporting metadata below each record.
Use spacing and headings before adding boxes.
Keep commands copyable. Preserve whitespace inside quoted arguments.
Wrap prose and paths by terminal cells. Preserve the original value in machine output.
Give empty results a useful next action based on available commands.

Pickers show the question, query, matching count, selected item, and valid keys.
Full-screen views keep scope and current state visible.
For large inventories, start with recommendations and counts. Open projects or folders before listing individual Skills.
Use words for findings. Label identical copies as duplicates and show their paths; separate symlinks from directory copies.
State when a filter changes only the view. An action review must still list every affected target.
Spend rows on the user's decision. Do not reserve space for decorative panels.
If a pane cannot show complete content, provide reachable scrolling or a detail view.
At smaller sizes, simplify the layout before hiding required information.
If the screen is unusable, block hidden actions and show the required size.

## Use colour with independent cues

Inherit the terminal's foreground and background for body text.
Share roles for brand, action, secondary information, success, warning, error, focus, and selection.
Use one action accent across screens. Reserve status colours for actual status.
Show ownership as a label, without implying safety or success.
Pair colour with words, markers, bold text, or inverse selection.
Make pane focus visible when colour is disabled.
Honor `NO_COLOR`, `TERM=dumb`, and existing output capability detection.
Never claim contrast across unknown terminal themes from one screenshot.

## Make state and input agree

Use explicit states for loading, browsing, editing, review, applying, completion, and failure.
Prefer tagged states carrying their required data over overlapping pending flags.
Keep filesystem and network work outside the input loop.
If operations can overlap, identify their results so stale work cannot replace the current view.
Derive available actions and footer hints from the same state rules.
Keep query editing separate from global shortcuts. Typing `q` in a query must remain possible.
Make Escape predictable: cancel editing or review before leaving the whole screen.
Preserve selection by stable identity when refreshing data.
Bound scrolling and expose position when the remaining content is otherwise unclear.
Provide discoverable help when the current footer cannot explain all controls.

## Explain waiting and outcomes

Show the operation, scope, elapsed time, and available cancellation action.
When the total is unknown, show activity and observed counts without a percentage.
When the total is known, show completed and total work.
Keep previous results readable during refresh when they remain relevant. Label them as previous results if refresh fails.
Keep errors visible until the user changes state or dismisses them.
Do not replace an error with a filter label or a generic loading message.
Describe partial completion explicitly. Preserve recovery and backup paths.

Before a destructive change, show the exact action, item, paths, and content replacement.
Require deliberate confirmation of that specific plan.
Discard stale activation keys before entering consent states. Preserve cancellation requests.
If writes cannot stop safely, defer exit and say so. Never display a cancellation key that does nothing.
Restore the terminal on success, failure, and cancellation.
After leaving the alternate screen, print a durable summary of completed changes and recovery paths.
Provenance describes source evidence. It never proves that instructions are safe.

## Verify the whole journey

Run focused behavior tests for changed transitions and regressions.
Use the real renderer for layout tests. Do not test source text or private implementation shape.
Inspect actual terminal captures, including colour and monochrome.
Cover normal and narrow sizes, short screens, Unicode names, long paths, and resize during confirmation.
Exercise loading, empty results, errors, filtering, review, cancellation, completion, and return to the shell.
Use disposable fixtures for migration, removal, and restoration.
Check plain and JSON output separately for escape sequences, stream changes, and exit codes.
Run the repository's required checks after integration.

Report what changed and what was exercised. Identify any live path that remains untested.
Do not treat passing snapshots as proof of keyboard behavior or terminal restoration.
