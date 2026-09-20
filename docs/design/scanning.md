---
type: Architecture Design
title: Parallel scanning and accounting
description: Proposed exact scanning pipeline, compact tree storage, and disk-usage semantics.
status: proposed
tags: [parallelism, filesystem, accounting]
---
# Implementation status

The [parent-ID implementation](/parent-ids.md) replaces the naive scanner.
It uses pinned `dua-core` 4.1.0 with completion-order results, directory-relative metadata, compact per-entry names, and direct directory-ID lookup.
Children may arrive before parents. Finalization resolves their integer IDs and uses pending child-directory counts for bottom-up aggregation.
There is no global path sort, parent-path hash map, or application-level per-entry collection lock.

The original implementation is preserved in `benchmarks/baseline/`.
UI integration and a contiguous name arena below remain proposals; current names use `OsString`.

# Scope and performance assumptions

This concept defines exact mode. [Estimation](/estimation.md) deliberately avoids collecting every file's metadata.
Rust and parallel traversal alone do not establish a speed advantage: ncdu already supports parallel scanning.
Compare against parallel ncdu and dua under the [verification plan](/verification.md).

Potential bottlenecks in the earlier design were tree construction on the UI thread, resolving every parent from the root path, and maintaining link records for every entry.
These are design concerns, not measured findings.

# Scanner selection

Use `dua-core` 4.1.0, requiring Rust 1.88 or newer.
Its API provides work-stealing traversal, metadata batches, and parent directory IDs.
The previous `ignore` scanner remains only in the frozen baseline.

Include hidden and gitignored entries, and never follow symbolic links.
Reuse collected metadata for sizes and link identity instead of requesting it again.
Bound workers with `--threads`; choose the default from measurements at counts such as 1, 2, 4, and 8 rather than CPU count alone.

Require parallel metadata collection within a single large directory, not only parallelism between directories.
Record any remaining serial enumeration limit.
Do not introduce a custom scheduler or async runtime in the initial implementation.

# Data flow

```text
Parallel enumeration and metadata collection
  -> Library result stream
  -> Tree owned by a dedicated aggregation thread
  -> Visible-directory view data
  -> UI thread
```

Separate tree construction from rendering.
Send only visible rows and progress to the UI, not copies of the complete tree.
Coalesce progress updates so slow rendering does not stop scanning.
Start with a maximum of ten screen updates per second, avoiding formatting and rendering per entry.
Directory totals remain provisional during scanning; retain the last available value when updating it is expensive.

Inspect library-internal job and result queues as well as application buffers when testing memory bounds.
An application-level bounded channel does not prove the whole pipeline has bounded buffering.
Notify the aggregation thread on cancellation and stop workers by dropping the traversal iterator.
Do not promise immediate cancellation of an OS filesystem call that is blocked.

# Tree and aggregation

Use `Order::Completion`.
In this library version on Linux, `Order::ParentFirst` collects a whole directory serially, causing a measured wide-directory regression.
Retain the emitted parent directory ID with each node and map directory IDs to node indices when directory entries arrive.
Resolve these integer references after traversal; no path lookup or placeholder tree nodes are needed.

Store names in a contiguous byte arena and keep offsets and lengths in nodes.
Keep parent IDs, types, sizes, and state in a node array; only directories need child lists.
This avoids persistent per-name allocations and per-file child-list storage.
Detect ID and name-arena capacity overflow.
Preserve non-UTF-8 bytes and reconstruct paths only when operations or error displays need them.

Register each entry's own size, add regular entries to their parents, then process directories with no pending child directories.
After contributing a directory total, decrement its parent's pending count and enqueue that parent when the count reaches zero.
This is an iterative dependency traversal independent of entry arrival order.
Avoid updating every ancestor for every file, which costs O(entries * depth).
Target O(N) final aggregation; measure name copying and hard-link comparisons separately.

Sort only directories being viewed and reuse their finalized display order.
Measure the first sort of a huge directory separately from scan time.

Aggregation remains a serial stage.
Reduce per-entry work first; partition aggregation only if measurements show it cannot keep up with traversal.

# Size semantics

On Linux, keep allocated bytes (`MetadataExt::blocks() * 512`) separate from apparent bytes (metadata length).
Include directory metadata and account for sparse files having different allocated and apparent sizes.
Count a symlink's own metadata only.
These metrics do not promise exclusive physical storage accounting for shared extents or compression.

Identify hard links by `(device, inode)`.
Only non-directory entries with at least two links enter the deduplication table.
Handle repeated directory traversal separately from file hard-link accounting.
Compare paths only among duplicate candidates, not by sorting every path in the scan.
After scanning, charge each identity to the observed root-relative path with the smallest bytewise ordering and mark the others as duplicates.
This makes totals independent of scheduling for a stable filesystem.
A link in another subtree may receive the charge, so per-directory totals need not match ncdu's attribution.

# Errors and boundaries

Fail clearly if the root cannot be opened.
Treat unreadable or vanished descendants as incomplete results, not successful zero-sized entries.
Detect integer overflow.
Do not promise snapshot consistency while files are changing.

`-x` prevents descent onto a different device.
See the [interface](/interface.md) for presentation and [verification](/verification.md) for edge cases.

# Citations

- [dua-core 4.1.0](https://docs.rs/dua-core/4.1.0/dua_core/): scheduling, ordering, and cancellation.
- [dua-core Entry](https://docs.rs/dua-core/4.1.0/dua_core/struct.Entry.html): parent IDs and collected metadata.
- [Rust MetadataExt](https://doc.rust-lang.org/std/os/unix/fs/trait.MetadataExt.html): blocks, device, inode, and link counts.
- [ncdu](https://dev.yorhel.nl/ncdu): parallel scanning since version 2.5.
- [ignore WalkBuilder](https://docs.rs/ignore/latest/ignore/struct.WalkBuilder.html): alternative parallel traversal API.
