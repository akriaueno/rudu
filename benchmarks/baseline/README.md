# Frozen naive baseline

This is the source and lockfile used for the original benchmark. Keep this
snapshot unchanged so comparisons remain reproducible without relying on an
uncommitted worktree or an ignored binary.

Build from the repository root:

```sh
CARGO_TARGET_DIR=.bench/baseline-target cargo build --release --locked --manifest-path benchmarks/baseline/Cargo.toml
```

Pass `.bench/baseline-target/release/rudu` as `--baseline` to
`scripts/benchmark.py`. The source hash matches the original baseline report;
a rebuild may have a different binary hash due to build paths or toolchains.
