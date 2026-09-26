# Contributing

1. Keep generic STEP/EXPRESS mechanics independent of IFC and application schemas.
2. Add or update a failing regression before production behavior changes.
3. Run `bash scripts/gate.sh`.
4. Keep capability claims aligned with executable tests; the EXPRESS frontend is currently structural and partial.

## Licensing contributions

Unless an explicitly signed agreement says otherwise, every contribution
submitted to this repository is licensed under `AGPL-3.0-or-later`. Submit only
work that you have the right to license. Identify third-party material and
preserve its license, attribution, and provenance.

## Releasing

Releases publish from CI through crates.io trusted publishing
(`.github/workflows/release.yml`); no one needs a crates.io token.

1. Bump `version` in the root `Cargo.toml`, run `cargo update -p openbim-step`,
   and date the `## [Unreleased]` changelog section as `## [x.y.z] - date`
   (with its compare link).
2. Run `bash scripts/gate.sh`, and `cargo semver-checks` against the last
   release to confirm the bump matches the change.
3. Merge to `main`, then push an annotated tag `vx.y.z` on that commit.

The workflow refuses a tag that is not on `main`, does not match the
manifest version, or has no changelog section; it gates the tagged commit,
publishes, and creates the GitHub release from the changelog. Re-running a
partly failed release skips what is already live. To rehearse, run the
workflow by hand with an existing tag: it gates and packages, and publishes
nothing.

crates.io trusts this repository, the file name `release.yml` and the
`release` environment (crate Settings -> Trusted Publishing); renaming
either needs the same change there.
