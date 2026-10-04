# vdm

## Rust API

`vdm_fyi::VdmCheckout::fetch` resolves HTTP, Git, and GitHub sources using the
same syntax as the CLI and returns a local file or directory:

```rust,no_run
# async fn example() -> anyhow::Result<()> {
let checkout = vdm_fyi::VdmCheckout::fetch("gh:owner/repo:templates@main").await?;
let path = checkout.path();
// Read or copy the templates while checkout is alive.
# Ok(())
# }
```

Run it within a Tokio runtime. Snapshots live in vdm's usual cache directory
and are deleted when the handle is dropped. Git objects remain in vdm's shared
local data directory. The API does not create project manifests, lockfiles, or
vendor directories, or resolve source-code dependencies. Git globs return a
directory containing matches at their repository-relative paths.

## Monorepos

Declare package directories in the root `vendor.toml`:

```toml
[monorepo]
pkgs = ["pkgs/*"]
```

Patterns are relative to the root manifest. Matching directories with a
`vendor.toml` are included; overlapping patterns include each package once.
Directories without a manifest are skipped until you add their first source.
Each package keeps its own `vendor.toml`, `vendor.lock.toml`, and `vendor/`.
Sources declared in the root manifest are included as well.

```sh
# Add a source to a package (creates its manifest if needed).
vdm --pkg ./pkgs/crate add gh:owner/repo:path/to/file.rs

# Install all included packages using their individual locks.
vdm install

# Update this source in every package that declares it.
vdm update gh:owner/repo:path/to/file.rs

# Update every declared source in every included package.
vdm update

# Limit installation or updates to a package.
vdm --pkg ./pkgs/crate install
vdm --pkg ./pkgs/crate update
```

`--pkg` is relative to the current working directory and cannot be combined
with `--manifest`. `install --offline` and `update --review` also work across
packages. A source-specific update skips packages without that source and
errors if no selected manifest declares it. Source matching uses normalized
source keys, just like single-package updates. Omitting the source updates
each declared source using its default upstream version, just as an explicit
update without a version does.
