"use strict";

// Release links remain usable without JavaScript or a successful API request.
(async () => {
  const repository = "https://github.com/Sophialovesmorrowind/OpenMWMergedLands";
  const api = "https://api.github.com/repos/Sophialovesmorrowind/OpenMWMergedLands";
  const status = document.getElementById("release-status");
  const version = document.getElementById("latest-version");
  const developmentStatus = document.getElementById("development-status");
  const platforms = [...document.querySelectorAll(".platform-download")];
  const required = platforms.flatMap((platform) => {
    const name = platform.dataset.package;
    return [name, `${name}.sha256sum.txt`, `${name}.sha512sum.txt`];
  });

  async function fetchJSON(path) {
    const response = await fetch(`${api}${path}`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error("GitHub request failed");
    return response.json();
  }

  async function findRelease() {
    try {
      const releases = await fetchJSON("/releases?per_page=20");
      for (const release of releases) {
        if (release.draft || release.prerelease || !release.tag_name ||
            /^(development|nightly)$/i.test(release.tag_name)) continue;
        const assets = new Set((release.assets || [])
          .filter((asset) => asset.state === "uploaded" && asset.size > 0)
          .map((asset) => asset.name));
        // Keep all six platforms and their checksums on one complete release.
        if (!required.every((name) => assets.has(name))) continue;
        const base = `${repository}/releases/download/${encodeURIComponent(release.tag_name)}`;
        for (const platform of platforms) {
          const url = `${base}/${platform.dataset.package}`;
          const button = platform.querySelector(".download-button");
          button.href = url;
          button.title = `Download Merged Lands ${release.tag_name} for ${button.textContent}`;
          platform.querySelector(".sha256").href = `${url}.sha256sum.txt`;
          platform.querySelector(".sha512").href = `${url}.sha512sum.txt`;
        }
        version.textContent = release.tag_name;
        status.hidden = true;
        return;
      }
      version.textContent = "Browse releases";
      status.textContent = "No complete release is available yet. Browse releases or choose a development build.";
      for (const platform of platforms) {
        const button = platform.querySelector(".download-button");
        button.href = `${repository}/releases`;
        button.title = `Browse Merged Lands releases for ${button.textContent}`;
        platform.querySelector(".platform-checksums").hidden = true;
      }
    } catch {
      status.textContent = "Release details are temporarily unavailable. Latest-release download links are still available.";
    }
  }

  async function findDevelopmentBuild() {
    try {
      const { workflow_runs: runs } = await fetchJSON(
        "/actions/workflows/release.yaml/runs?branch=main&event=push&status=success&per_page=5",
      );
      for (const run of runs) {
        if (run.head_branch !== "main" || run.event !== "push" ||
            run.conclusion !== "success" || !Number.isSafeInteger(run.id)) continue;
        const { artifacts } = await fetchJSON(`/actions/runs/${run.id}/artifacts?per_page=100`);
        const available = new Map(artifacts
          .filter((artifact) => !artifact.expired && artifact.size_in_bytes > 0 &&
            Number.isSafeInteger(artifact.id))
          .map((artifact) => [artifact.name, artifact]));
        const artifactName = (platform) =>
          platform.dataset.package.replace(/^merged_lands-/, "merged-lands-").replace(/\.zip$/, "");
        if (!platforms.every((platform) => available.has(artifactName(platform)))) continue;
        for (const platform of platforms) {
          const artifact = available.get(artifactName(platform));
          const link = platform.querySelector(".development-download");
          link.href = `${repository}/actions/runs/${run.id}/artifacts/${artifact.id}`;
          link.title = "Download the latest successful main build (GitHub sign-in required)";
        }
        return;
      }
      developmentStatus.textContent = "Dev links open GitHub Actions. Download artifacts from a successful build; GitHub sign-in is required.";
    } catch {
      developmentStatus.textContent = "Dev builds are available from GitHub Actions and require sign-in to download.";
    }
  }

  await Promise.allSettled([findRelease(), findDevelopmentBuild()]);
})();
