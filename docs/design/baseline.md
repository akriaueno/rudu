---
type: Benchmark Report
title: Naive Rust baseline
description: Implemented exact parallel scanner and reproducible warm-cache comparison against ncdu.
status: implemented
tags: [baseline, benchmark, rust]
---
# Historical baseline

This report describes the frozen snapshot in `benchmarks/baseline/`, not the current scanner.
See [parent-ID implementation](/parent-ids.md) for the replacement and paired comparisons.

# Implemented scope

The first implementation is a non-interactive Linux CLI, deliberately preceding estimation and bound-based algorithms.
It uses `ignore` for parallel traversal and standard-library collections for storage and aggregation.
The scanner retains full paths and per-entry sizes, collects results through a shared append lock, sorts all paths, resolves parents with a directory map, deduplicates hard links, and aggregates bottom-up.
This is a full retained-tree baseline, not a streaming total-only program.

Hidden and gitignored files are included, symlinks are not followed, and directories contribute their own metadata sizes.
Hard-link attribution is deterministic by path bytes.
Partial scans return exit code 2 with error details; invalid roots and fatal failures return 1.
The CLI supports `--threads`, `-x`, `--list`, `--apparent-size`, and `--scan-only`.

The global collection lock, full path allocations, and O(N log N) path sort are deliberate baseline limitations.
No sampling, lower/upper-bound search, TUI, or deletion is implemented.
The proposed optimized layout in [scanning](/scanning.md) remains future work.

# Reproduction

From the repository root:

```sh
cargo build --release
python3 scripts/benchmark.py --ncdu .bench/tools/ncdu \
  --real /home/akira/ghq/github.com/akriaueno/t3code
```

ncdu was downloaded from the official x86_64 static-binary link into `.bench/tools`, without a system-wide installation.
Raw measurements, commands, binary hashes, source hash, lockfile hash, versions, and host metadata are saved in `docs/benchmarks/baseline.json`.
The real-tree argument is optional and should name an existing, unchanged project on another machine.

# Method

Use five timed runs per tool and worker count (1, 4, 8), in shuffled order with a fixed seed.
Both programs retain their scanned entries; ncdu uses `--ignore-config -0 --quit-after-scan`.
Measure process launch through exit with Python's monotonic clock, including aggregation and teardown.
Collect maximum RSS using GNU time and report the median of the per-run maxima.
Run accounting verification and one warm-up for each configuration before timing.
These are warm-cache measurements on a shared host, not isolated microbenchmarks or cold-cache results.

Synthetic cases contain 128-byte regular files:

- `spread-100k`: 100,000 files across 1,000 directories.
- `wide-100k`: 100,000 files in one directory.
- `spread-1m`: 1,000,000 files across 1,000 directories.

Allocated totals are checked against GNU du.
Apparent totals are checked against GNU du plus directory metadata lengths, since GNU du omits those lengths in apparent-size mode while this scanner includes them.
Synthetic allocated bytes, apparent bytes, and entry counts are also checked against an ncdu JSON export outside the timed runs.
All measured worker counts must return identical rudu totals.
The real tree's totals must match before and after timing; this detects aggregate changes but is not a filesystem snapshot.

# Results

Measured on an AMD Ryzen 5 7600 (6 cores, 12 logical CPUs), Samsung SSD 990 PRO 4TB, ext4, Linux 7.0.0-31-generic.
Compiler: Rust 1.90.0; default Cargo release profile; locked `ignore` 0.4.30; ncdu 2.9.1 official static binary.
The official static download was 2.9.1 although the website also listed 2.9.2 source; no claim about untested versions is made.
The timings include the GNU time wrapper and subprocess overhead, which matters most for the smallest case.
The benchmark timestamp and exact artifact hashes are recorded in the raw JSON.

| Dataset | Threads | rudu median (s) | ncdu median (s) | rudu / ncdu time | rudu peak RSS (MiB) | ncdu peak RSS (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| spread-100k | 1 | 0.2589 | 0.1514 | 1.71 | 24.0 | 3.8 |
| spread-100k | 4 | 0.1052 | 0.0394 | 2.67 | 26.5 | 3.7 |
| spread-100k | 8 | 0.0791 | 0.0246 | 3.22 | 27.1 | 3.4 |
| wide-100k | 1 | 0.2960 | 0.1473 | 2.01 | 53.6 | 3.5 |
| wide-100k | 4 | 0.1766 | 0.1464 | 1.21 | 47.1 | 3.7 |
| wide-100k | 8 | 0.1308 | 0.1458 | 0.90 | 37.6 | 3.7 |
| spread-1m | 1 | 2.5051 | 1.3401 | 1.87 | 210.3 | 33.8 |
| spread-1m | 4 | 1.0012 | 0.3516 | 2.85 | 221.4 | 33.5 |
| spread-1m | 8 | 0.8278 | 0.2125 | 3.90 | 227.6 | 33.5 |
| t3code | 1 | 0.0654 | 0.0434 | 1.51 | 7.2 | 1.2 |
| t3code | 4 | 0.0254 | 0.0119 | 2.14 | 7.6 | 1.0 |
| t3code | 8 | 0.0188 | 0.0075 | 2.51 | 7.9 | 0.8 |

A time ratio below 1 favors rudu; above 1 favors ncdu.
The raw JSON includes every run and min/max times; the table does not imply statistical significance for small differences.
The synthetic cases passed ncdu export parity for both size metrics and entry count.
The real project passed GNU du allocated-size parity and adjusted apparent-size parity, with unchanged aggregate totals before and after the timed runs.

# Interpretation

The naive implementation does not meet the twofold speed target.
It is slower and substantially more memory-hungry than ncdu on the distributed synthetic trees.
For the single wide directory, the 8-worker rudu median is lower, while ncdu's times change little with worker count in this dataset.
This does not establish a general speed advantage or prove the cause of either tool's behavior.
Full paths, collection locking, and path sorting are visible optimization candidates, not profiled explanations.

Keep this result as the exact-scan reference before adding algorithms.
Next implement positive threshold discovery from deduplicated observed lower bounds, and use an upper bound only when its assumptions are valid.
Measure time to qualifying candidates and entries inspected separately from time to complete exact totals; do not present the smaller query as a like-for-like full-scan speedup.

# Validation and limits

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` pass.
Tests cover independent metadata-based totals, parallel equivalence, hard links across directories, sparse files, symlink cycles and dangling links, hidden and ignored files, non-UTF-8 names, permission failures, invalid roots, overflow, and invalid arguments.
Permission checks ran as a non-root user.

Cross-device boundaries, concurrent mutation stress, and a TUI are not validated by this baseline.
Shared physical extents and reclaimed space are outside the accounting contract.
No claim against dua is made; this iteration compares only ncdu, as requested.

# Citations

- [ncdu official downloads and benchmark guidance](https://dev.yorhel.nl/ncdu).
