## What

<!-- One paragraph. What changed and why. -->

## Proof

<!-- Commands you ran, their output, and screenshots for visible changes. -->

## Checks

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `bun run check` and `bun run build` in `web/`
- [ ] Contract regenerated and committed (`cargo run -p openjam-api --bin openjam-openapi`)
- [ ] `docker compose up` works from a fresh clone

Refs #
