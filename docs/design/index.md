---
okf_version: "0.1"
---
# rudu design

A fast Rust disk-usage CLI with ncdu-style browsing, parallel traversal, and an optional estimation mode.
Finding large, disposable development directories, including completed LLM-generated worktrees, is a motivating use case.
The product operates on filesystem paths; it does not manage Git workflows or decide whether development is complete.
The exact scanner now uses [parent IDs and dependency-based aggregation](/parent-ids.md).
The [naive baseline](/baseline.md) is preserved as a benchmark snapshot.
Interactive browsing, estimation, and threshold search remain proposals.

This bundle's root is `docs/design/`. Internal links starting with `/` resolve from that root.

- [Parent-ID implementation](/parent-ids.md): current exact scanner and paired performance results.
- [Performance analysis](/performance-analysis.md): measured bottlenecks and controlled experiments.
- [Naive baseline](/baseline.md): implemented behavior and ncdu comparison.
- [CLI and interface](/interface.md): scope, commands, navigation, and terminal behavior.
- [Parallel scanning and accounting](/scanning.md): exact scanning, tree storage, and size semantics.
- [Capacity estimation](/estimation.md): threshold discovery, adaptive scan allocation, sampling limits, and selective refinement.
- [Verification plan](/verification.md): correctness, performance targets, and measurement conditions.
- [Change log](/log.md)

# Provenance

The user requested a fast Rust alternative to ncdu with parallel traversal, an approximately twofold speed improvement, and consideration of estimation with roughly 10% error tolerance.
Architecture choices and additional acceptance thresholds are proposals.
Development rules live in `AGENTS.md`; product design lives in this bundle.
External sources are linked in each concept's Citations section.
