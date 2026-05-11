# Contributing to Lodestone

Thanks for your interest in Lodestone — a local SDLC + code knowledge graph
inspired by GitLab's Knowledge Graph. Contributions of any size are welcome:
bug reports, language extractors, query endpoints, MCP tools, docs.

## Getting set up

```bash
git clone https://github.com/jintukumardas/lodestone.git
cd lodestone

docker compose up -d                       # NATS + ClickHouse
cargo build --workspace
cargo test --workspace
```

You'll need:
- Rust 1.86 or newer (`rustup show`).
- Docker / Docker Compose for the NATS + ClickHouse stack.
- Roughly 2 GB of disk for the ClickHouse image and target/ build output.

## Project layout

The workspace is split by responsibility — see `README.md` for the full
diagram. When you change something, the crate to look at is usually obvious
from the area:

| Area | Crate |
|---|---|
| Wire types, IDs, NATS subjects | `crates/lodestone-core` |
| Tree-sitter walking and code extraction | `crates/lodestone-indexer` |
| SDLC event ingestion (issues / MRs) | `crates/lodestone-sdlc-emitter` |
| HTTP query layer over ClickHouse | `crates/lodestone-api` |
| MCP tools surfaced over stdio | `crates/lodestone-mcp-server` |
| Database schema (storage, NATS sources, MVs) | `clickhouse/migrations/` |

## How to contribute

1. **Open an issue first for non-trivial changes.** A short discussion saves
   rework — especially for new node/edge kinds, schema changes, or new tools.
   Bug fixes and small docs PRs can skip this step.
2. **Branch from `main`.** Use a descriptive branch name (`feat/python-indexer`,
   `fix/walker-utf8`, `docs/clarify-finals`).
3. **Keep commits focused.** One logical change per commit. The existing
   history is the model: a one-line subject in the form
   `area: imperative summary`, then a body explaining the why.
4. **Add tests where it's cheap to.** Parser changes should land with a
   tree-sitter unit test in `parse.rs`. ID and serialization changes should
   land with a stability test. Endpoint changes can be exercised via a curl
   one-liner in the PR body.
5. **Run before pushing:**
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
6. **Open a PR against `main`.** In the description, link the issue (if any),
   summarize the user-facing change, and call out anything reviewers should
   pay attention to (schema migrations, breaking API changes, new dependencies).

## Good first issues

If you're looking for a way in:

- **Add a tree-sitter language.** Wire up another grammar in
  `lodestone-indexer/src/parse.rs`; the match arms for `function_item`,
  `struct_item`, etc. are a template.
- **Cross-file call resolution.** Today `calls` edges hash on the bare callee
  name (`<repo>:fn:<name>`) and dangle across files. A real resolver would
  walk `use` statements and module paths.
- **Incremental indexing.** Only re-emit nodes/edges for files whose mtime or
  content hash changed since the last run.
- **More query endpoints.** "Files most-changed in MRs touching X", "issues
  whose closing MRs touch a given module", etc.
- **Schema migrations tool.** Today the migrations rely on Docker's
  init-db hook. A small migration runner would help re-runs and upgrades.

## Reporting bugs

Open an issue with:
- What you ran (commands + relevant args).
- What you expected, what happened.
- Output of `cargo --version`, `docker --version`, and the relevant logs
  (set `RUST_LOG=info` or `debug` to get more out of any binary).

## Licensing of contributions

By submitting a pull request, you agree that your contribution is licensed
under the same MIT License that covers the rest of the project. See `LICENSE`.

## Contact

Questions, ideas, or interest in collaborating? Reach out to
**Jintu Kumar Das** at jintukumardas@gmail.com or open a discussion on the
GitHub repo.
