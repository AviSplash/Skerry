# Working on Skerry

## Every change ships as a release

Each merged PR or update goes out as a new release:

1. Bump the patch version (for example 1.1.5 → 1.1.6) in `Cargo.toml` and `apps/skerry-app/tauri.conf.json`. Then build once so `Cargo.lock` follows, and update the download links in `README.md`. Do this in the PR itself or in a version PR right after it.
2. Once that's merged, run **Actions → Release** on `main` with the new version (for example `v1.1.6`). See [docs/releasing.md](docs/releasing.md).
3. Wait until all three releases (Windows, macOS, Linux) are published and the workflow is green.

## Checks before pushing

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p skerry-core -p skerry-platform`
