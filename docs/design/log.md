# Design change log

## 2026-09-21

- **Directory batches**: Adopt ncdu-inspired directory scheduling, FD-relative libc metadata calls, and arena-backed names. Merge results during scanning and compare ncdu with paired confidence intervals. Record the flat-directory regression from the previous checkpoint.

- **Parent-ID implementation**: Replace full-path sorting and parent-path lookup with direct IDs and dependency-based aggregation. Use completion-order traversal to retain parallel metadata collection in wide directories. Freeze the naive source and benchmark it alongside the new scanner and ncdu.

- **Performance analysis**: Measure scan phases, isolate collection and path-storage changes, compare metadata APIs, and inspect syscall counts and ncdu source. Preserve the original implementation and binary.

- **Baseline implementation**: Add a naive parallel Rust scanner and a reproducible ncdu comparison before implementing bound-based algorithms.

- **Algorithm revision**: Separate relative-error estimation, top-k identification, and threshold discovery. Add a worst-case information bound, monotone observed lower bounds, adaptive work allocation, and conditions for valid stopping rules.

- **Scope correction**: Keep rudu filesystem-focused. Remove proposed Git workflow management, completion detection, and worktree-specific commands. Retain development worktrees as an example workload.
- **Language**: Convert all documentation to English.
- **Estimation**: Add [capacity estimation](/estimation.md), empirical accuracy targets, and selective exact refinement.
- **Performance targets**: Target approximately twice parallel ncdu's speed; evaluate dua parity and a 0.8 elapsed-time stretch target separately.
- **Architecture**: Prioritize evaluating `dua-core`, parent IDs, separate aggregation and rendering, and contiguous name storage.
- **Verification**: Add parallel ncdu and dua comparisons, explicit conditions, and `--scan-only`.
- **Creation**: Separate product design from `AGENTS.md` into this OKF bundle.
- **Status**: The headless exact baseline is implemented. The TUI, estimation, and bound-based algorithms remain proposed.
