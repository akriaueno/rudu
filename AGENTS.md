# Development rules

## Working agreements
- Write all commit messages in English, using concise imperative subjects.
- Write repository documentation, including all files under `docs/`, in English.
- Preserve unrelated changes. Do not commit or publish unless requested.
- Keep product scope, architecture, data structures, and dependency choices in the OKF design bundle at [docs/design/index.md](docs/design/index.md), not in this file.
- Update the relevant design concept when implementation changes an agreed design decision. Distinguish proposals from implemented behavior.

## Implementation
- Implement only the requested scope. Prefer existing code and standard-library facilities before adding dependencies or abstractions.
- Understand the affected flow and callers before editing. Fix defects at their shared cause.
- Keep changes focused. Do not introduce speculative configuration or infrastructure.
- Preserve input validation, error handling, security, and accessibility when simplifying code.
- Document deliberate shortcuts with a `ponytail:` comment describing the limitation and when to replace them.

## Verification
- For Rust code changes, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
- Leave focused runnable checks for nontrivial logic using Rust's built-in tests; avoid tests that merely mirror the implementation.
- Verify relevant edge cases and error paths against the documented requirements.
- Claim correctness and performance improvements only with evidence. Report what was checked and any remaining limitations.
- Validate the OKF bundle after changing design documents.

<!-- CODEGRAPH_START -->
## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root), reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **MCP tool** (when available): `codegraph_explore` answers most code questions in one call — the relevant symbols' verbatim source plus the call paths between them, including dynamic-dispatch hops grep can't follow. Name a file or symbol in the query to read its current line-numbered source. If it's listed but deferred, load it by name via tool search.
- **Shell** (always works): `codegraph explore "<symbol names or question>"` prints the same output.

If there is no `.codegraph/` directory, skip CodeGraph entirely — indexing is the user's decision.
<!-- CODEGRAPH_END -->
