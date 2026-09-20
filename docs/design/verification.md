---
type: Verification Plan
title: Verification plan
description: Correctness checks and separate performance criteria for exact scanning and estimation.
status: proposed
tags: [testing, performance]
---
# Evaluation tracks

Evaluate [estimation](/estimation.md) on time to useful directory ranking and actual error.
Evaluate threshold discovery separately on time to the requested number of qualifying candidates, correctness of observed lower bounds, and inspected-entry fraction.
Include cases where no candidate qualifies, since early positive stopping cannot help those cases.
Evaluate exact mode independently using the targets below.
Development worktrees are representative input data, not a requirement for Git integration.

# Implementation sequence

1. Implement a naive, parallel exact Rust baseline and compare it with ncdu before introducing new algorithms.
2. Preserve its code and reproducible measurements as the reference; see the [baseline report](/baseline.md).
3. Introduce observed lower bounds and valid upper bounds where available, measuring threshold discovery separately from complete accounting.
4. Evaluate sampling and storage/scanner optimizations against the baseline rather than changing every layer at once.
5. Add the TUI after validating the computational benefit; measure rendering overhead separately.

# Correctness

Use Rust's built-in tests and small temporary trees covering regular files, empty directories, hidden and gitignored files, sparse files, symlinks, and hard links.
Compare totals with independently calculated metadata-based expectations.
Place hard links in different subtrees and verify identical attribution across worker counts.

Cover permission errors, disappearing entries, invalid roots, non-UTF-8 names, terminal control characters, and arithmetic overflow.
Run permission checks without root privileges.
Report filesystem-boundary behavior as unverified when the required environment is unavailable.

Verify cancellation under full queues and traversal of deep trees without call-stack exhaustion.
Check navigation, metric switching, input during scanning, partial-result reporting, and terminal restoration in a real terminal.

# Exact-mode performance targets

The naive baseline and parent-ID scanner are implemented and measured separately. High performance remains a target for subsequent iterations.
The primary target is approximately twice the speed of parallel ncdu.
On a predefined set of large local-SSD trees, target a geometric mean of per-case median elapsed-time ratios `rudu / ncdu` no greater than 0.5.
Against dua, initially target parity or better, with 0.8 as a stretch target.
Do not promise twice dua's speed or combine its outcome with the ncdu target.

If any target case regresses beyond a 1.1 elapsed-time ratio against a comparator, qualify claims against that comparator by workload.
Initially target peak RSS within 1.2 times dua and TUI overhead within 5% of the equivalent headless run.
The twofold ncdu target interprets the user's speed goal; the dua, memory, and UI thresholds are proposals.
None is a forecast or guarantee.
Report failures and causes without changing the dataset after seeing results.

# Measurement conditions

Use identical datasets and accounting semantics for rudu with one and multiple workers, parallel ncdu, and dua.
Separate default-setting results from comparisons with equal worker counts.
Try counts such as 1, 2, 4, and 8, extending them when appropriate for the hardware.
Record tool versions, build settings, worker counts, entry counts, elapsed time, peak RSS, storage medium, filesystem, and cache conditions.

Measure from process start through final accounting, excluding user input delays.
Exact `rudu --scan-only` must build the same tree as interactive mode.
Competitor modes that do not retain a tree are supplemental results, not equivalent interactive-workload comparisons.
The ncdu website suggests `-0 --quit-after-scan` for benchmarking; verify flags against the tested version.
Check whether dua provides an equivalent build-and-exit mode and report any mismatch in measurement scope.

Run each condition at least five times with varied execution order, reporting medians and spread.
Do not call a threshold met when the difference is obscured by variability.
Separate warm-cache and cold-cache results; do not clear host-wide caches without authorization.
Report cold-cache behavior as unmeasured if it cannot be tested.

# Datasets

| Case | Intended stress |
| --- | --- |
| Real projects with many small files | Metadata collection and insertion |
| One directory with many files | Metadata parallelism and initial display sort |
| Many small directories | Scheduling and parent-ID management |
| Deep hierarchies | Depth-dependent path and aggregation costs |
| Many hard links | Deduplication and attribution |
| HDD or network filesystem | I/O waits and concurrency regressions |

Start with 100,000 entries and test memory and scaling at one million or more.
Record generation parameters and distinguish synthetic data from real trees.
Report HDD and network results separately from SSD acceptance criteria.
Limit performance claims to measured environments.

# Bottleneck analysis

Measure traversal with discarded results, tree construction, final accounting, and rendering as separate configurations.
Overlapping pipeline stages mean elapsed-time differences are not direct CPU-time measurements for each stage.
As needed, inspect CPU time, metadata syscall counts, allocation counts, peak RSS, and result-queue backlog.
Keep traced runs separate from timing comparisons because instrumentation has overhead.

A twofold speedup requires removing half the original elapsed time.
In a simplified serial model, an unchanged 70% portion limits speedup to about 1.43 even if the rest becomes free.
For a parallel pipeline, identify critical work and waits rather than deriving that fraction from CPU samples alone.
Do not assume aggregation and rendering improvements can double performance when traversal is shared.

Remove redundant metadata requests first.
If tree processing dominates, revisit layout and parent IDs.
The baseline already issues Linux `statx`; the [performance analysis](/performance-analysis.md) supports testing directory-FD-relative access rather than changing syscall names alone.
Defer `io_uring`, custom allocators, and custom work-stealing until evidence justifies them.

# Citations

- [ncdu](https://dev.yorhel.nl/ncdu): parallel scanning and benchmark options.
- [dua](https://github.com/Byron/dua-cli): existing parallel disk-usage tool.
