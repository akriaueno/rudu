---
type: Implementation Report
title: Parent-ID scanning and aggregation
description: Exact scanning without global path sorting, with paired comparisons against the frozen baseline and ncdu.
status: implemented
tags: [implementation, performance, parent-ids]
---
# Change

This report describes checkpoint `00bfe90`. The [directory-batch scanner](/directory-batches.md) supersedes it; measurements below remain historical.

Replace the `ignore` scanner with pinned `dua-core` 4.1.0 and raise the minimum Rust version to 1.88.
Store each entry's name, sizes, directory flag, and parent ID instead of a full path and optional hard-link identity on every node.
Use collected metadata directly; on Linux the walker obtains it through directory entries rather than resolving full paths for each file.
The application no longer has a global per-entry collection mutex, global path sort, or parent-path hash map.

Keep a dense table from the walker's directory IDs to retained node indices.
Resolve parent IDs with array lookups after collection.
Add non-directory sizes to their parents, then process directories whose child-directory count is zero.
After contributing a directory, decrement its parent's pending count and enqueue the parent when ready.
This handles children arriving before parents in O(N + D) time with O(D) aggregation bookkeeping, where N is entries and D is directories.
The retained tree still requires O(N) memory.

Hard-link identities are recorded only for entries with multiple links.
Compare reconstructed bytewise paths only within a duplicate identity group, zero the losing entries, then aggregate.
Attribution remains independent of arrival order.
Hard-link path comparisons and name storage are additional costs beyond the linear aggregation pass.

# Traversal ordering decision

An initial parent-first version removed sorting but regressed on a directory containing 100,000 files.
Source inspection showed that `dua-core` 4.1.0 on Linux uses whole-directory serial metadata collection for `Order::ParentFirst`.
The final version uses `Order::Completion`, which distributes metadata batches across workers.
Dependency counts replace the assumption that parents appear before children.
The intermediate measurements are retained in `docs/benchmarks/parent-first.json`; they are not the final implementation's results.

# Paired benchmark

Same host and datasets as the [baseline](/baseline.md): AMD Ryzen 5 7600, Samsung SSD 990 PRO, ext4, warm caches.
Use five shuffled runs per tool and worker count, measuring process start through teardown.
The frozen original binary, current release binary, and ncdu 2.9.1 run in the same measurement session.
Exact source, lockfile, and binary hashes plus every run are in `docs/benchmarks/parent-ids.json`.
The old source and lockfile are preserved in `benchmarks/baseline/`.

| Dataset | Workers | Baseline ms | Current ms | ncdu ms | Baseline/current speedup | Current/baseline peak RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| spread-100k | 1 | 251.4 | 189.3 | 148.4 | 1.33x | 11.0 / 24.0 |
| spread-100k | 4 | 96.2 | 53.9 | 40.8 | 1.79x | 11.1 / 26.5 |
| spread-100k | 8 | 78.0 | 33.8 | 24.5 | 2.31x | 11.0 / 27.1 |
| wide-100k | 1 | 288.6 | 182.8 | 143.4 | 1.58x | 10.8 / 53.5 |
| wide-100k | 4 | 167.8 | 62.5 | 146.6 | 2.68x | 10.7 / 47.8 |
| wide-100k | 8 | 127.6 | 51.2 | 143.4 | 2.49x | 10.7 / 40.5 |
| spread-1m | 1 | 2488.1 | 1754.8 | 1334.1 | 1.42x | 86.6 / 210.2 |
| spread-1m | 4 | 1002.6 | 484.5 | 368.4 | 2.07x | 87.0 / 221.7 |
| spread-1m | 8 | 829.4 | 308.5 | 218.7 | 2.69x | 87.4 / 227.0 |
| t3code | 1 | 62.8 | 47.9 | 41.4 | 1.31x | 4.4 / 7.3 |
| t3code | 4 | 23.9 | 14.1 | 11.2 | 1.69x | 4.2 / 7.7 |
| t3code | 8 | 18.6 | 9.8 | 7.5 | 1.89x | 4.3 / 7.9 |

All figures are medians; RSS is the median of per-run peak measurements.
The small real-project timings are sensitive to process-startup noise and shared-host activity.

At eight workers the million-file case improves from about 829 ms to 309 ms (2.69x), with peak RSS dropping from 227 MiB to 87 MiB.
The wide-directory case improves from about 128 ms to 51 ms (2.49x), avoiding the parent-first regression.
The million-file ncdu result is still about 219 ms, so the current scanner remains roughly 1.41x slower there.
The approximately twofold target against parallel ncdu is not met across the benchmark suite.

These end-to-end gains combine parent-ID aggregation, shorter retained names, a different walker, directory-relative metadata, and removal of the application's collection lock.
Do not attribute the entire improvement solely to parent IDs.

# Correctness and validation

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and all three Rust tests pass.
- A deterministic test feeds a grandchild file before its directories and verifies exact totals and reconstructed paths.
- Tests preserve metadata-based totals, cross-directory hard-link attribution, non-UTF-8 names, sparse files, hidden files, ignored files, symlinks, permission failures, and worker-count equivalence.
- Synthetic allocated bytes, apparent bytes, and entry counts match ncdu's JSON export and the frozen implementation.
- GNU du allocated totals and directory-adjusted apparent totals match all benchmark cases.
- Additional CLI comparisons match the frozen binary's `--list` output for both size metrics at 1, 4, and 8 workers, including a 64-level tree, hard links, hidden files, and escaped names.
- Partial/fatal error exit codes and SIGINT termination were checked.
- The real tree's aggregate totals are unchanged before and after timing; this is not a filesystem snapshot guarantee.

# Remaining limits

The CLI remains non-interactive and exact. No estimation, threshold pruning, deletion, or TUI is introduced.
Names still have individual `OsString` allocations; an arena is not part of this change.
Cross-device boundary behavior has not been integration-tested on a dedicated mount fixture.
Metadata errors include the entry path; enumeration errors from the walker carry only scan-root context because that API does not expose their entry path.
The walker uses 32-bit directory identifiers and does not support an unlimited number of directories.

# Reproduction

```sh
cargo build --release
CARGO_TARGET_DIR=.bench/baseline-target cargo build --release --locked --manifest-path benchmarks/baseline/Cargo.toml
python3 scripts/benchmark.py --ncdu .bench/tools/ncdu \
  --baseline .bench/baseline-target/release/rudu \
  --real /home/akira/ghq/github.com/akriaueno/t3code \
  --output docs/benchmarks/latest.json
```

The real project argument is optional. A rebuild of the baseline can change its binary hash because of compiler or build-path differences; the snapshot source hash matches the original report.
The benchmark defaults to `latest.json` to preserve historical evidence.

# Citations

- [dua-core 4.1.0](https://docs.rs/dua-core/4.1.0/dua_core/): public traversal and directory-ID API.
- The locked crate's `src/lib.rs`, `run_job`, `read_dir_parent_first`, `read_dir_parallel`, and `Entry::from_dir_entry`: inspected to establish Linux traversal ordering and metadata behavior.
