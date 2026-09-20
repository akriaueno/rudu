---
type: Interface Design
title: CLI and interface
description: Proposed commands, browsing behavior, and terminal handling for rudu.
status: proposed
tags: [cli, tui, scope]
---
# Implementation status

The [current scanner](/directory-batches.md) implements the non-interactive exact CLI with `--threads`, `-x`, `--scan-only`, `--list`, and `--apparent-size`.
The interactive interface and `--estimate` described below remain proposals.

# Scope

Start with Linux and read-only disk-usage browsing for arbitrary filesystem paths.
Support both exact accounting and optional [capacity estimation](/estimation.md).
Development worktrees are ordinary directories for this tool; Git registration, merge status, agent activity, and completion detection are outside the product scope.
Deletion, export/import, and other platforms remain later scope decisions.
Full ncdu option compatibility is not an initial goal.

# Command

```text
rudu [PATH] [--threads N] [--apparent-size] [-x] [--scan-only] [--estimate]
```

| Argument | Behavior |
| --- | --- |
| `PATH` | Directory to inspect; defaults to the current directory |
| `--threads N` | Positive worker count; accepts 1 for serial comparisons |
| `--apparent-size` | Initially display apparent bytes instead of allocated bytes |
| `-x` | Do not descend into a different filesystem |
| `--scan-only` | Run without rendering, print totals and error counts, and exit |
| `--estimate` | Estimate directory totals from samples; does not guarantee 10% error |

In exact mode, `--scan-only` builds the same tree and performs the same accounting as interactive mode.
In estimation mode, it produces estimates and sampling metadata without constructing a complete file tree.
These modes must be identified separately in benchmark results.

Use `clap` for arguments and `ratatui` with `crossterm` for the terminal interface, subject to checking versions and Rust compatibility during implementation.

# Interaction

Display entries by descending size, with names breaking ties deterministically.
Use up/down to select, Enter to enter a directory, left to go to its parent, and `a` to switch size metrics.
In estimation mode, offer exact refinement for the selected directory; browsing into an unmaterialized directory triggers its scan.
As a proposed extension, offer threshold discovery for a user-chosen size B and number of qualifying directories.
Its result is `at least B`, not a total-size estimate or a certified largest-directory ranking.
Keep CLI syntax for this extension undecided until the prototype validates the benefit.
Use `q` or Ctrl-C to cancel and exit.

Display processed and failed entry counts during scanning.
Distinguish estimated, provisional, fully scanned, and incomplete totals.
Show estimates such as `~12 GiB | estimated | metadata sampled: 8%`, without unsupported confidence intervals.
Escape terminal control characters and preserve non-UTF-8 paths internally.

Restore terminal state on normal exit, errors, and panic unwinding.
Forced termination cannot guarantee restoration.
Interactive mode requires a TTY; `--scan-only` also works without one.

See [scanning](/scanning.md) for data flow and [verification](/verification.md) for acceptance checks.
