---
type: Implementation Report
title: Directory batches and arena-backed names
description: An ncdu-inspired exact scanner with directory scheduling and paired performance evidence.
status: implemented
tags: [implementation, performance, ncdu, parallelism]
---
# Changes

Checkpoint `00bfe90` contains the previous [parent-ID implementation](/parent-ids.md).
The current scanner replaces `dua-core` with a small directory queue, standard-library scoped workers, and `libc` bindings.
It adopts ncdu's directory scheduling, directory-FD-relative metadata lookup, and bulk name storage ideas; this is an independent Rust implementation rather than a source translation.
No approximation is used.

Each worker opens one directory, reads names with `getdents64`, and obtains one `fstatat(AT_SYMLINK_NOFOLLOW)` result per entry.
A small safe callback wrapper contains the unsafe calls and closes the directory on normal return or unwinding.
It validates record lengths and NUL termination before using names; it never casts a possibly unaligned directory record.
Each worker reuses its 32 KiB enumeration buffer across directories.
`O_NOFOLLOW` also rejects a queued directory replaced by a final-component symlink before opening.
This is not a snapshot guarantee or protection against every ancestor rename race.

Names retain their original bytes in an arena; nodes contain ranges into it.
Workers merge completed directory results under one collection mutex while other workers continue scanning.
They flush after at least 256 entries, at a directory boundary, and flush the remainder before exiting; a very large single directory can exceed that batch size.
Publishing groups of 16 discovered subdirectories lets peers start before a large parent finishes.
A worker keeps one child locally for depth-first progress, reducing shared-queue traffic.
The calling thread is one of the workers; explicit `--threads` skips default CPU-quota detection.
Idle workers sleep on a condition variable, using an outstanding-directory count to detect completion.
There is no per-file queue, per-file lock, or persistent per-name allocation.
The shared collection lock can still limit scaling at higher worker counts; the current results only cover up to eight workers.

An intermediate version joined worker-owned arrays after scanning.
One instrumented million-file run took 205.8 ms in traversal, 26.6 ms in merging, and 3.2 ms in aggregation.
Moving merges into the directory pipeline removes that terminal copy phase, rather than avoiding any metadata queries.
These diagnostic timings are one observation, not the acceptance benchmark.

Final aggregation retains the checkpoint's parent-ID resolution, deterministic hard-link attribution, checked sizes, and iterative directory dependency counts.
Metadata and enumeration errors carry their affected paths.
The CLI still retains the complete tree and reports partial scans with exit code 2.

The intermediate `readdir` implementation is retained as measurements in `docs/benchmarks/directory-batches.json` (30 rounds).
It reached parity on the million-file case but was about 10% slower on the real project at eight workers.
Tracing that project exposed 1,798 `fstat` and 3,586 `fcntl` calls in the intermediate scanner, versus none of those calls in ncdu.
The native enumeration path removes the directory-stream setup checks, while batching reduces collection-lock contention.
The traces are in `docs/benchmarks/strace-directory-real.txt` and `strace-ncdu-real.txt`.
They are diagnostic evidence only; traced elapsed times are not benchmark results.

# Measurement method

`docs/benchmarks/native-batches.json` records 50 shuffled rounds for each tool at 1, 4, and 8 workers before the final wakeup correction.
All twelve comparisons meet the 5% slowdown criterion in that run.
The final source broadcasts when publishing 16 directory jobs, so a single group can wake all idle workers.
`docs/benchmarks/native-final.json` repeats 50 rounds at eight workers on the final source.
Both use the same warm-cache host and datasets as the checkpoint.
Each round contains both tools; comparisons use paired log elapsed-time ratios, 10,000 deterministic bootstrap resamples, and percentile 95% confidence intervals.
A ratio below 1 favors rudu.
Timing includes process launch, scanning, complete-tree retention, accounting, and teardown.
Raw samples, source and binary hashes, hardware, filesystem, versions, and commands are retained.

The engineering acceptance margin is a maximum 5% slowdown: the upper confidence bound must be below 1.05.
Practical equivalence additionally requires the lower bound above 0.95.
A confidence interval containing 1 alone does not prove equivalence.
Intervals are per comparison, not simultaneous family-wide guarantees, and shared-host warm-cache results do not establish cold-cache or other-filesystem behavior.

# Results

Final source, eight workers. Times and peak RSS are medians; the ratio is the geometric mean of paired round ratios.

| Dataset | Rounds | rudu ms | ncdu ms | Paired ratio (95% CI) | Peak RSS MiB (rudu / ncdu) |
| --- | ---: | ---: | ---: | --- | ---: |
| spread-100k | 50 | 24.64 | 25.30 | 0.988 (0.964–1.016) | 8.5 / 3.4 |
| wide-100k | 50 | 142.13 | 144.97 | 0.983 (0.976–0.991) | 13.4 / 3.7 |
| spread-1m | 50 | 212.28 | 219.31 | 0.969 (0.959–0.981) | 58.6 / 33.5 |
| t3code | 500 | 7.61 | 7.58 | 0.999 (0.988–1.010) | 4.2 / 0.8 |

The first final-source real-project run (50 rounds) had ratio 1.032 with CI [1.001, 1.074], so it did not establish the 5% slowdown criterion.
Its measurements remain in `native-final.json`.
Because millisecond-scale timings were noisy, a fixed 500-round follow-up was declared before running it, without changing the scanner or estimator.
That follow-up is `native-real-confirmation.json`; its CI is [0.988, 1.010], establishing practical equivalence in that session.
Do not erase the initial inconclusive result or treat these intervals as an environment-independent guarantee.

The three synthetic comparisons and the real-project confirmation meet the 5% slowdown criterion.
This establishes measured ncdu parity or better for these warm-cache workloads, not a twofold advantage or equivalence on every filesystem.
The final source, lockfile, and executable hashes were checked against both final-source reports.

# Tradeoffs and validation

Directory-level scheduling matches the compared ncdu architecture but gives up the checkpoint's parallel stat batches within a single flat directory.
The previous wide-100k result was about 51 ms with eight workers; the new implementation is near ncdu's roughly 140 ms.
This is a deliberate scope change for ncdu parity, not a claim that every workload improved over the checkpoint.
A future wide-directory optimization can dispatch bounded stat batches without restoring per-entry messages.

Correctness checks cover sparse files, symlinks, non-UTF-8 and control-byte names, hidden and ignored files, cross-directory hard links, child-before-parent completion, and parallel equivalence.
The integration test creates 40 directories to exercise early publication while parent batches are still outstanding, and 140 maximum-length names to cross enumeration-buffer boundaries.
A focused parser test rejects truncated, zero-length, oversized, and unterminated records.
Additional CLI comparisons against the checkpoint cover a 64-level tree, both list metrics, permission errors, fatal errors, and `/dev -x` mount boundaries at one and eight workers.
SIGINT termination was checked. A traced 16-directory fixture confirmed all eight workers issue metadata calls after the wakeup correction.
The final real-project syscall summary is `docs/benchmarks/strace-native-real.txt`.
Synthetic totals and entry counts match ncdu's export; allocated totals and directory-adjusted apparent totals match GNU du.
Real-tree totals are checked before and after timing, without claiming a snapshot.

Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `python3 scripts/test_benchmark.py`.
Validate this OKF bundle with the skill's validator.
The statistics check covers ratio direction, exact equivalence, and mismatched pairs.
The unsafe filesystem wrapper is Linux-specific; other targets and architectures are not validated.
TUI, deletion, estimation, and bounds remain unimplemented.

# Reproduction

```sh
cargo build --release --locked
python3 scripts/benchmark.py --ncdu .bench/tools/ncdu --runs 50 --threads 8 \
  --real /path/to/an/unchanging/project \
  --output docs/benchmarks/new-native-batches.json
```

Omit `--threads 8` to compare all default benchmark counts (1, 4, and 8).
Use `--case real --runs 500 --threads 8 --real PATH` to reproduce the focused confirmation.
Do not overwrite historical reports or run timing jobs concurrently.

# Citations

- [Linux getdents manual](https://man7.org/linux/man-pages/man2/getdents.2.html): the `linux_dirent64` field layout and buffer-return contract.

- [ncdu 2.9.1 source archive](https://dev.yorhel.nl/download/ncdu-2.9.1.tar.gz): inspected `src/scan.zig` (`statAt`, directory worker queue), `src/mem_sink.zig` (thread arenas), and `src/model.zig` (compact retained entries).
- Repository source `src/main.rs` and benchmark harness `scripts/benchmark.py` provide the implemented behavior and comparison procedure.
