# rudu

An exact, parallel disk-usage CLI for Linux (Rust 1.88 or newer).

```sh
cargo build --release
./target/release/rudu /path/to/directory --threads 4 --list
```

The summary reports allocated bytes, apparent bytes, entries (including the
root), and errors. `--list` prints immediate children in descending size order;
`--apparent-size` switches that ordering and its displayed size. Names are
escaped. `-x` stays on one filesystem. `--scan-only` explicitly selects the
current headless behavior. Use `--` before paths beginning with a dash.

Hidden and gitignored files are included. Symlinks are counted but not followed;
the root must be a directory, not a symlink. Hard links are counted once across
the scan, attributed to the first path in bytewise order. Directory metadata is
included. Usage is not a guarantee of space reclaimed by deletion.

Exit codes are 0 for a complete scan, 1 for a fatal error, and 2 for a partial
scan. The default worker count is the available parallelism capped at 8;
`--threads 1` provides a serial-worker comparison. Ctrl-C terminates the process.

The scanner uses `dua-core` for parallel, directory-relative metadata collection.
It stores entry names and parent IDs rather than full paths, and aggregates
through directory dependency counts without a global path sort. Only duplicate
hard links require path reconstruction. TUI browsing, sampling, bound-based
search, and deletion are not implemented yet.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Reproduce the benchmark

Download ncdu separately, then run:

```sh
cargo build --release
CARGO_TARGET_DIR=.bench/baseline-target cargo build --release --locked --manifest-path benchmarks/baseline/Cargo.toml
python3 scripts/benchmark.py --ncdu /path/to/ncdu \
  --baseline .bench/baseline-target/release/rudu \
  --output docs/benchmarks/latest.json
# Optional: add --real /path/to/an/unchanging/project
```

Requires Python 3, GNU `du`, `/usr/bin/time`, `findmnt`, `lsblk`, and `lscpu`.
The script creates 1.2 million 128-byte files under `.bench/data` (several GiB
of allocated storage). It reuses completed fixtures, compares totals, and runs
five warm-cache measurements per tool and thread count. It does not drop caches
or modify the optional real project. A real project must remain unchanged.

See the [parent-ID implementation report](docs/design/parent-ids.md),
[frozen baseline report](docs/design/baseline.md), and
[OKF design bundle](docs/design/index.md).
