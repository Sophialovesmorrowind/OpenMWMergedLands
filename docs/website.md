# Merged Lands website

The website follows GOCoverify's Soupault layout: a banner, six platform downloads,
checksum and development links, README and changelog pages, and automatic light/dark styling.
Source files are in `web/site/`, the shared layout is `web/templates/main.html`, and
`web/soupault.toml` configures the build.

## Build and preview

Install `cmark`, then build and check the site from the repository root:

```sh
sh web/build.sh
python3 ci/check_site.py web/build
node --test ci/test_downloads.mjs
python3 -m http.server 8000 --directory web/build
```

Open `http://localhost:8000/`. The output is also usable under the GitHub Pages
project path because Soupault converts internal links and assets to relative URLs.
Generated pages and downloaded build tools are ignored by Git.

The build script downloads checksum-verified Soupault 4.8.0 on Linux x86-64.
For another platform, install that Soupault release and set `SOUPAULT` to its
executable path. `cmark` can also be placed in `web/.tools/`.

## GitHub Pages

In the repository's **Settings → Pages → Build and deployment**, set **Source**
to **GitHub Actions**. Keep the `github-pages` environment. If it has deployment
branch restrictions, allow `main`.

`.github/workflows/pages.yaml` checks the site on pull requests and deploys it
when website changes are pushed to `main`. It can also be run manually on `main`.
Its deploy job uses the existing `github-pages` environment and publishes the
deployment URL. The executable build/release workflow remains separate.

The expected project URL is
`https://sophialovesmorrowind.github.io/OpenMWMergedLands/`.

## Download behavior

The static HTML links directly to the latest GitHub release ZIPs and checksum files.
The optional browser script checks public release metadata and selects the newest
non-draft, non-prerelease release with all six ZIPs and all twelve nonempty,
uploaded checksum files. A release still being uploaded cannot split the buttons
between versions. API failures preserve the static latest-release links.

Development links resolve to matching platform artifacts from the latest complete,
successful `main` push build. Expired or incomplete artifact sets fall back to an
older complete build. GitHub requires sign-in to download Actions artifacts;
without metadata, these links open the workflow's build history.

## Attribution

The layout and styling were adapted from the local GOCoverify website. Its GPL
notice and the MIT notices for Sakura, Normalize.css, MOMW Mod Template, and MOMW
Configurator are published under the website's `licenses/` path. The Merged Lands
program retains its existing MIT license.
