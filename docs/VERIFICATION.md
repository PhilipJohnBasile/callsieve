# Verification notes

Documentation refresh: September 10, 2026.

## Reproduce the focused checks

```bash
cargo test --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Local result

The current source has published CI coverage for default and optional embedding configurations. The initial macOS fresh-checkout attempt could not complete because native build scripts were terminated or stalled before program startup. Use the linked workflow result for hosted validation.

## Hosted checks

[Workflow and current runs](https://github.com/PhilipJohnBasile/callsieve/actions/workflows/ci.yml). Inspect the commit and individual jobs when using a run as evidence; a successful earlier run does not validate later source changes.

## Release and coverage scope

The latest existing release at the start of this refresh was v0.5.0. The README documents both that release and the source-checkout path.
