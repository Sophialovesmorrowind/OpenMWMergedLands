# Dependency security checks

The build and release workflow requires a dependency audit as well as the normal
formatting, lint, test, and packaging checks. The audit also runs daily and can
be started manually. It checks the entire committed `Cargo.lock`, without
filtering out other operating systems or optional dependency entries.

The gate uses `cargo-audit 0.22.2` with `--deny warnings`: vulnerabilities and
informational warnings, including unsoundness and unmaintained dependencies,
fail the job. There are no advisory exceptions. A database or registry fetch
failure fails the job rather than counting as a clean scan. The JSON report is
saved as the `cargo-audit` workflow artifact, including when an audit fails.

To run the same check locally:

```sh
cargo install cargo-audit --version 0.22.2 --no-default-features
cargo audit --deny warnings
```

The scanner version is pinned, but its installation resolves current compatible
dependencies instead of using the older lockfile published with the tool.

Dependabot proposes Cargo dependency and GitHub Actions updates weekly. Review
and test those changes before merging them; it does not merge them automatically.

## November 2026 dependency updates

The lockfile at commit `187ab53` contained two versions covered by current
RustSec advisories:

| Package | Previous version | Updated version | Advisory |
| --- | --- | --- | --- |
| `anyhow` | `1.0.102` | `1.0.103` | [RUSTSEC-2026-0190](https://rustsec.org/advisories/RUSTSEC-2026-0190.html): unsound mutable error downcast |
| `crossbeam-epoch` | `0.9.18` | `0.9.21` | [RUSTSEC-2026-0204](https://rustsec.org/advisories/RUSTSEC-2026-0204.html): invalid pointer dereference during pointer formatting |

`anyhow` is a direct dependency; its manifest minimum is also raised.
`crossbeam-epoch` is used transitively through Rayon and the TES3 parser. The
patched transitive version is retained in `Cargo.lock`, which release builds
use with `--locked`.

These advisories are included even though they have no CVE aliases. The
application does not call `anyhow::Error::downcast_mut` directly; the dependency
updates remove the affected versions without relying on that observation.

The manual version review used the complete RustSec database tree at
[`263a4d9`](https://github.com/RustSec/advisory-db/tree/263a4d951a0867d4bddc29dddd6d59427c78f9a7),
dated 2026-11-02. All eleven advisory records for the locked package names were
reviewed, including informational records. The audit job uses a freshly fetched
database on every run.

A clean dependency scan establishes that the scanned versions have no reported
matches in that database at that time. It does not prove the absence of unknown
bugs, vulnerable application code, or weaknesses in a previously published binary.
Existing release downloads require a rebuild and a new release to receive these
dependency updates.
