---
type: Algorithm Design
title: Capacity estimation
description: Output-sensitive discovery of large directories using observed lower bounds and adaptive sampling.
status: proposed
tags: [estimation, sampling, performance]
---
# Purpose

Help users locate large directories without waiting for complete size accounting.
Completed development worktrees are one example, alongside build outputs and dependency directories.
Accept arbitrary paths and let the user decide which directories are disposable.
Git integration and automatic completion detection are not required.

# Separate the computational objectives

There are three different queries:

1. Estimate every directory total within a relative error target.
2. Identify the largest k directories, without estimating the smaller ones precisely.
3. Find some directories whose usage exceeds a threshold B.

The third query is often sufficient for cleanup and permits stronger early results than universal relative-error estimation.
The second is useful for navigation but requires valid upper bounds to certify that an unseen competitor cannot win.
Do not describe a provisional ranking as a certified top-k result.

# Information lower bound

Assume a stable filesystem accessed through ordinary directory enumeration and per-entry metadata queries, with no precomputed subtree totals or useful prior size bounds.
Two trees can return identical observations for all inspected entries while an uninspected entry is small in one tree and arbitrarily large in the other.
An algorithm returning the same estimate cannot satisfy 10% relative error for both.
Thus a worst-case deterministic guarantee requires inspecting all potentially decisive entries: linear metadata-query work in the worst case.

For a uniform sample of m out of N files, the probability of observing one particular dominant file is m/N.
Seeing that file with 95% probability requires sampling at least 95% of the files.
This example explains why a universal high-confidence guarantee can erase the hoped-for sampling savings.
Parallelism can reduce elapsed time but not this information requirement.

# Monotone lower bounds for threshold discovery

For each candidate directory, maintain L, the sum of allocated bytes for distinct observed file identities plus observed directory metadata.
For a stable tree, nonnegative sizes imply L <= S, where S is that candidate's fully scanned usage.
As soon as L >= B, report `at least B` and suspend that candidate's scan unless the user requests its total.
No extrapolation or probabilistic assumption is needed for this positive result.

Deduplicate observed hard links before increasing L.
Compute each candidate's standalone usage independently: do not transfer attribution between candidates and accidentally reduce a previously reported lower bound.
Candidates may share storage, so their lower bounds must not be added as guaranteed combined usage or reclaimed bytes.
Changing files invalidate the stable-tree argument; expose scan-time observations rather than snapshot guarantees.

A candidate below B remains unresolved until fully scanned or given a valid upper bound.
Unreadable descendants do not prevent reporting an observed lower bound, but prevent declaring the candidate smaller than B.
This mode accelerates finding qualifying directories, not proving that every other directory is small.

# Adaptive allocation of scan work

Use a fixed worker pool and bounded batches of metadata work per candidate.
Give every candidate an initial budget, then prioritize candidates near the user's decision boundary and likely to yield useful qualifying results quickly.
Reserve a share of work for less-explored candidates so a poor initial estimate cannot permanently starve them.
Pause a candidate once it passes B, and stop the discovery query once the requested number of candidates has qualified.
The UI may keep exact refinement available on demand.

Use pilot samples as scheduling hints, not as permission to discard candidates with no valid upper bound.
This is inspired by good-arm identification: spend observations on the decision the user needs rather than estimating every option equally well.
Filesystem candidates have different population sizes, correlated files, enumeration costs, and hard links; published bandit guarantees do not automatically apply.
Start with this simple scheduler rather than implementing a generic bandit framework.

# Sampling proposal

Separate name enumeration from metadata collection.
Divide the requested tree into strata such as immediate subtrees, enumerate names, and collect size metadata for a random subset of regular files in each stratum.
Estimate each stratum's total from its population count and sample mean.
Inspect small strata completely and retain metadata operations needed to traverse directories or identify entry types.
Include hidden and ignored files without following symlinks.

This reduces size-related metadata calls and avoids retaining a complete file tree, but still enumerates names.
It may offer little improvement when enumeration dominates or the filesystem requires metadata calls to determine entry types.
An engine that fetches all metadata before returning entries cannot deliver this saving.
Evaluate metadata-skipping support before reusing the exact scanner.

Names such as `node_modules`, `target`, and `.next` can help define strata; they must not imply fixed sizes.
An identical lockfile does not imply identical installed files or build outputs.
Do not introduce persistent caching initially.
If caching is added later, parent directory mtime alone cannot validate unchanged descendant contents.

# Estimation and stopping rules

For a fully enumerated stratum h with N_h entries and a uniform fixed-size sample, estimate its path-summed size by `N_h * sample_mean_h`.
Sum these estimates across strata, adding components that were measured exactly.
For a fixed sample design this estimates the path sum without requiring equal file sizes; it does not deduplicate unobserved hard links.
Use uniform reservoir sampling rather than taking the first files returned by the filesystem.
Sampling a directory uniformly and then one of its files uniformly is not a uniform sample of all files.

For candidate i with a valid total-size interval [L_i, U_i], a proposed top-k set A is certified only if:

```text
min(L_i for i in A) > max(U_j for j outside A)
```

Without a useful upper bound on unobserved contributions, U remains effectively uninformative and this rule cannot stop early.
Do not substitute the largest sampled file for a bound on unsampled file sizes.

For a valid positive interval [L, U], an estimate x has at most epsilon relative error for every possible total in that interval only when:

```text
(1 - epsilon) * U <= x <= (1 + epsilon) * L
```

At epsilon = 0.1, such an x exists only if U/L <= 1.1/0.9.
These interval rules are conditional: they do not manufacture valid bounds from an arbitrary sample.
If confidence intervals are added, state their distribution or boundedness assumptions and use time-uniform coverage or a preallocated error budget across inspection rounds and candidates.
Repeatedly checking ordinary fixed-sample 95% intervals until one looks narrow does not preserve 95% coverage.
The initial implementation uses observed lower bounds and empirically evaluated estimates; it does not advertise certified probabilistic intervals.

Adaptive sample expansion also needs an implementation budget.
Retaining every unsampled path costs O(N) memory; re-enumerating to draw additional samples costs I/O.
Start with a fixed pilot reservoir and exact refinement, and measure these costs before adding multiple adaptive sampling rounds.
Do not claim that reservoir sampling avoids the initial O(N) name enumeration.

# Accuracy contract

Treat approximately 10% error as an empirical target on representative datasets, not a guarantee for arbitrary trees or a statistical confidence statement.
A single unsampled large file can dominate the total.
Low observed sample variance alone cannot certify accuracy.

Display estimates separately from fully scanned and incomplete results.
Include sample count, enumerated population, and observation time.
Do not display an unsupported `+/-10%` interval.
Prioritize exact refinement for large, closely ranked, or user-selected directories.
Rankings remain provisional until refined.

Sampling cannot exactly deduplicate hard links.
A simple estimator targets the sum across paths, which may exceed deduplicated usage.
Observed multiple links require refinement; not observing them does not prove they are absent.
Evaluate error against the deduplicated exact reference and restrict the estimator's advertised applicability if shared-file datasets fail the accuracy target.

# Usage versus reclaimable space

Directory usage is not a guarantee of space reclaimed by deletion.
External hard links, shared extents, snapshots, and open files can retain storage.
Even exact scanning does not resolve every reclaimability condition.
Label the metric as usage rather than guaranteed savings.
Do not follow references or symlinks into external shared stores.

# Evaluation

Separate tuning datasets from held-out evaluation datasets.
Propose a target of absolute relative error at most 10% in at least 95% of held-out cases.
Report absolute error for zero-byte references.
Include single huge outliers, uneven build artifacts, hard links, and sparse files, and report these cases separately.
Repeat with multiple random seeds and measure missed large-directory candidates as well as size error.

Measure time to first display, estimated ranking, first threshold-qualified candidate, requested number of qualified candidates, selected-directory refinement, and complete exact results separately.
Test threshold discovery against complete reference scans on stable data: every positive result must truly exceed B.
Report inspected-entry fractions and worst-case behavior when all candidates are smaller than B.
Initially target half the time to useful candidate selection compared with scanning every candidate exactly.
Include required enumeration and refinement costs.
Do not report a sampled result as a like-for-like exact-scan speedup.

# Repeated-use alternative

A maintained directory-size index can shift work from queries to filesystem updates.
It does not accelerate the first scan, and correctness requires handling renames, hard links, missed events, downtime, and reconciliation.
Defer this larger feature until repeated queries justify it; mtime-only caching is not a substitute.

# Citations

- [Katz-Samuels and Jamieson, 2020](https://proceedings.mlr.press/v108/katz-samuels20a/katz-samuels20a.pdf), Sections 1 and 1.1: distinguish identifying useful options from certifying an optimum; inspiration rather than a filesystem-specific guarantee.
- [Howard et al., confidence sequences](https://arxiv.org/html/1810.08240): time-uniform inference and its assumptions for sequential observations.

- [GNU du](https://www.gnu.org/s/coreutils/manual/html_node/du-invocation.html): size definitions and hard-link accounting.

See [exact scanning](/scanning.md) for the reference semantics and [verification](/verification.md) for common benchmark conditions.
