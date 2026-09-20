---
type: Performance Analysis
title: Why the naive baseline is slower
description: Measured phase costs, controlled variants, metadata lookups, and source-level differences from ncdu.
status: measured
tags: [profiling, baseline, performance]
---
# Conclusion

The baseline does more per-entry path and allocation work than ncdu, and leaves substantial work serial after traversal finishes.
The measured bottleneck is not adding file sizes, and removing the shared collection lock alone does not close the gap.
At the time of this analysis, production source and the original release binary were unchanged and their hashes were verified.
The analyzed source is now frozen in `benchmarks/baseline/`; the [parent-ID implementation](/parent-ids.md) is the subsequent change.

# Method and evidence

Use the existing warm-cache million-file fixture (1,000 directories) on the same host as the [baseline report](/baseline.md).
Run five repetitions in shuffled order and compare medians.
Build isolated source variants under `.bench/profile` using the same lockfile and release profile.
Run the existing tests on each variant and verify every timed variant returns exactly the original summary.
Store raw runs in `docs/benchmarks/profile.json` and metadata experiments in `docs/benchmarks/metadata.json`.

Hardware-counter profiling was unavailable because `perf_event_paranoid` is 4; no host settings were changed.
Use coarse phase timers, controlled implementation variants, syscall counts, and source inspection instead.
These establish stage costs and experiment effects, not instruction-level attribution.

# Where the time goes

Eight-worker phase timings from the instrumented baseline:

| Phase | Median milliseconds |
| --- | ---: |
| Traversal, metadata, and result collection | 408.9 |
| Global full-path sort | 181.8 |
| Parent-path lookup and hard-link registration | 173.9 |
| Bottom-up aggregation and map cleanup | 2.1 |
| Dropping the retained result | 47.5 |

The instrumented total is 832.8 ms, versus 816.6 ms for the original and 215.9 ms for ncdu in this experiment.
The phase medians are not an exact additive decomposition of the median process time; startup, output, and the timing wrapper also contribute.
Sorting and parent linking together cost roughly 356 ms, more than the entire ncdu run.
Even zero-cost traversal would leave a substantial serial tail in this implementation.

At one worker, collection takes 2118.5 ms; at eight it takes 408.9 ms.
Parallel traversal helps, but does not parallelize the full-path sort or parent-map pass.
The summation pass itself is only about 2 ms, so optimizing integer addition is not a useful first step.

# Controlled variants

Each variant retains exact accounting and the full entry set. No early stopping or sampling is involved.

| Variant, eight workers | Median ms | Change from instrumented baseline | Median peak RSS MiB |
| --- | ---: | ---: | ---: |
| phase_baseline | 832.8 | +0.0% | 227.1 |
| worker_local | 765.3 | -8.1% | 239.9 |
| relative_paths | 703.9 | -15.5% | 121.2 |
| local_relative | 648.3 | -22.2% | 133.6 |

`worker_local` collects entries in each visitor and appends them under the shared lock when the visitor is dropped.
It reduces per-entry lock traffic, but improves total time by only about 8% and increases peak memory through temporary vectors and merge capacity.
It is evidence of a contributing cost, not a reason to adopt this exact buffering scheme.

`relative_paths` stores only paths relative to the scan root; metadata calls still use the original full paths.
Parent linking falls from about 174 ms to 83 ms, sorting from 182 ms to 130 ms, and result destruction from 48 ms to 28 ms.
Its collection stage is slightly slower because stripping path prefixes also costs work.
The improvement is therefore primarily in retained representation and downstream path processing, not cheaper filesystem lookups.
The combined variant remains substantially slower than ncdu.

# Filesystem lookup costs

A separate Rust diagnostic reads the same million regular files without retaining a tree or performing parent lookup, sorting, or hard-link accounting.
It uses static directory chunks across workers and compares two APIs:

- `fs::symlink_metadata(entry.path())`: construct and resolve a full path.
- `entry.metadata()`: use the directory entry's metadata operation.

A small syscall trace on this host confirms the first calls `statx(AT_FDCWD, "/full/path/...", ...)`, while the second calls `statx(directory_fd, "filename", ...)`.
Both use the same no-follow flags and `STATX_ALL` request in that trace.
This comparison combines the effect of avoiding path construction with directory-relative resolution; it does not separate those two savings.

| API | Workers | Median ms |
| --- | ---: | ---: |
| full | 1 | 1807.7 |
| full | 8 | 308.0 |
| entry | 1 | 1338.4 |
| entry | 8 | 220.5 |

At eight workers the directory-entry approach reduces this microbenchmark's elapsed time by approximately 28%.
These are not complete scanner timings and must not be substituted for an end-to-end result.
The inspected ncdu source uses `fstatat(parent.fd, name, ...)`, consistent with the same directory-relative strategy.

# Syscall counts

Separate `strace -f -c` runs on the 100,000-file distributed fixture, with eight workers:

| Tool | Primary per-entry metadata calls | futex calls |
| --- | ---: | ---: |
| Original rudu | 101,009 statx | 1,786 |
| Worker-local variant | 101,009 statx | 662 |
| ncdu | 101,000 newfstatat, plus one root stat | 76 |

The fixture has 101,001 entries including directories.
Both tools make approximately one metadata lookup per entry; systematic double-stat behavior is not the explanation.
The original also makes about 1,000 directory `fstat` calls, not another 100,000 file lookups.
Lock syscall counts fall in the worker-local variant, consistent with less synchronization, but futex activity also includes library scheduling and joins.
Trace timing and percentages are heavily perturbed and sum activity across threads; they are not production wall-time percentages.
The reports are saved as `docs/benchmarks/strace-{rudu,local,ncdu}.txt`.

# Memory and ncdu's representation

The baseline's `Node` occupies 88 bytes, and its observed vector capacity is 1,048,576 entries: 88 MiB before allocating any path strings.
Typical file paths in this fixture occupy 87 bytes, compared with 18 bytes for root-relative paths.
Per-path allocations, allocation overhead, traversal state, and temporary buffers add to that floor.
Measured peak RSS is roughly 227 MiB for the instrumented baseline versus 34 MiB for ncdu.
Relative storage reduces the baseline variant to roughly 121 MiB, but does not make its node representation compact.

The ncdu 2.9.1 source uses a compact regular-file record containing packed flags/blocks, size, and a next pointer, with the filename stored inline.
It uses per-thread arena allocation in `mem_sink.zig`, rather than an individually allocated full path for every retained file.
Normal files are accumulated into their directory as they arrive; completed child-directory totals propagate to parents.
It does not need the baseline's final global full-path sort and parent-path reconstruction pass.
These source differences explain plausible mechanisms for the observed memory and phase differences; their individual end-to-end contributions have not all been isolated.

# Why the single wide directory behaved differently

The inspected ncdu worker loop reads entries and calls `scanOne` within one directory, distributing newly discovered child directories to other workers.
With one directory full of regular files there are few directory jobs to distribute.
The `ignore` walker queues non-directory entries as work too, allowing rudu's metadata callback to run across workers.
That scheduling difference is consistent with the wide-directory result in the baseline report.
It does not imply that rudu is faster on typical trees.

# Recommended next steps

1. Keep this baseline and its hashes as the reference.
2. Remove the global path sort and parent-path hash pass by assigning parent IDs during traversal; resolve deterministic attribution only for actual hard-link groups.
3. Use directory-relative metadata access and compact retained names, avoiding repeated path construction.
4. Batch results or use per-directory ownership without retaining an extra full-tree copy during merge.
5. Evaluate lower-bound threshold discovery separately: its benefit is inspecting fewer entries, not merely processing the same entries faster.

These are evidence-based priorities, not implemented fixes or promised speedups.
Do not attribute the difference to Rust versus Zig: both algorithmic work and data representation differ.

# Reproduction

Generate the fixtures with `scripts/benchmark.py`, then run from the repository root:

```sh
python3 scripts/profile_baseline.py
python3 scripts/profile_metadata.py
strace -f -c -o .bench/profile/strace-rudu.txt .bench/baseline/rudu .bench/data/spread-100k --threads 8 --scan-only
strace -f -c -o .bench/profile/strace-ncdu.txt .bench/tools/ncdu --ignore-config -0 --quit-after-scan -t 8 .bench/data/spread-100k
strace -f -c -o .bench/profile/strace-local.txt .bench/profile/target/release/worker_local .bench/data/spread-100k --threads 8 --scan-only
```

Do not run tracing concurrently with untraced benchmarks.
The scripts require ncdu, existing fixtures, cached Cargo dependencies, and GNU time.
The baseline profiler reads `benchmarks/baseline/` and builds a saved baseline binary if necessary; reruns write `profile-latest.json` without overwriting the historical measurements.
They keep generated variants under `.bench` and do not change production source.

# Citations

- [ncdu 2.9.1 source archive](https://dev.yorhel.nl/download/ncdu-2.9.1.tar.gz): `src/scan.zig` (`statAt`, `Thread.run`), `src/mem_sink.zig` (`Thread.arena`, `Dir.addStat`, `Dir.final`), and `src/model.zig` (`Entry`, `File`).
- Locked local `ignore` 0.4.30 source: `src/walk.rs`, `Worker::run_one` and `Worker::generate_work`, for per-entry work scheduling.
