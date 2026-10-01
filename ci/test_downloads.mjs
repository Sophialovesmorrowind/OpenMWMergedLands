import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const script = readFileSync(new URL('../web/site/js/releases.js', import.meta.url), 'utf8');
const template = readFileSync(new URL('../web/templates/main.html', import.meta.url), 'utf8');
const repository = 'https://github.com/Sophialovesmorrowind/OpenMWMergedLands';
const api = 'https://api.github.com/repos/Sophialovesmorrowind/OpenMWMergedLands';
const workflow = `${repository}/actions/workflows/release.yaml?query=branch%3Amain`;
const packages = ['linux-amd64', 'linux-arm64', 'macos-amd64', 'macos-arm64', 'windows-amd64', 'android-arm64']
  .map(platform => `merged_lands-${platform}.zip`);
const assets = packages.flatMap(name => [name, `${name}.sha256sum.txt`, `${name}.sha512sum.txt`])
  .map(name => ({name, size: 100, state: 'uploaded'}));
const release = {tag_name: '1.1.1', draft: false, prerelease: false, assets};
const run = {id: 123, head_branch: 'main', event: 'push', conclusion: 'success'};
const artifacts = packages.map((name, index) => ({
  id: 1000 + index,
  name: name.replace('merged_lands-', 'merged-lands-').replace('.zip', ''),
  expired: false,
  size_in_bytes: 100,
}));

async function render({releases = [release], runs = [], builds = {}} = {}) {
  const controls = packages.map(name => {
    const latest = `${repository}/releases/latest/download/${name}`;
    const elements = {
      '.download-button': {href: latest, textContent: name},
      '.sha256': {href: `${latest}.sha256sum.txt`},
      '.sha512': {href: `${latest}.sha512sum.txt`},
      '.platform-checksums': {hidden: false},
      '.development-download': {href: workflow},
    };
    return {dataset: {package: name}, elements, querySelector: selector => elements[selector]};
  });
  const elements = {
    'release-status': {hidden: false},
    'latest-version': {textContent: 'Latest release'},
    'development-status': {textContent: 'Dev builds require sign-in.'},
  };
  const requests = [];
  const fetch = async (url, options) => {
    requests.push(url);
    assert.equal(options.headers.Accept, 'application/vnd.github+json');
    let response;
    if (url === `${api}/releases?per_page=20`) response = releases;
    else if (url.startsWith(`${api}/actions/workflows/release.yaml/runs?`)) {
      response = runs instanceof Error || runs.ok === false ? runs : {workflow_runs: runs};
    } else {
      const match = url.match(/\/actions\/runs\/(\d+)\/artifacts\?per_page=100$/);
      assert.ok(match, `Unexpected API request: ${url}`);
      response = {artifacts: builds[match[1]] || []};
    }
    if (response instanceof Error) throw response;
    return response.ok === false ? response : {ok: true, json: async () => response};
  };
  await vm.runInNewContext(script, {fetch, document: {
    getElementById: id => elements[id],
    querySelectorAll: selector => {
      assert.equal(selector, '.platform-download');
      return controls;
    },
  }});
  return {controls, elements, requests};
}

function expectRelease(result, version) {
  for (const control of result.controls) {
    const url = `${repository}/releases/download/${encodeURIComponent(version)}/${control.dataset.package}`;
    assert.equal(control.elements['.download-button'].href, url);
    assert.equal(control.elements['.sha256'].href, `${url}.sha256sum.txt`);
    assert.equal(control.elements['.sha512'].href, `${url}.sha512sum.txt`);
    assert.equal(control.elements['.platform-checksums'].hidden, false);
  }
  assert.equal(result.elements['latest-version'].textContent, version);
  assert.equal(result.elements['release-status'].hidden, true);
}

test('all six platform ZIPs and checksum links work without JavaScript', () => {
  assert.equal([...template.matchAll(/class="platform-download"/g)].length, 6);
  assert.ok(!template.includes('class="platform-checksums" hidden'));
  for (const name of packages) {
    assert.ok(template.includes(`data-package="${name}"`));
    for (const file of [name, `${name}.sha256sum.txt`, `${name}.sha512sum.txt`]) {
      assert.ok(template.includes(`href="${repository}/releases/latest/download/${file}"`));
    }
  }
  assert.equal(template.split(`href="${workflow}"`).length - 1, 6);
});

test('a complete release updates all six downloads and their checksums', async () => {
  expectRelease(await render(), release.tag_name);
});

test('incomplete newest uploads and empty or unfinished assets fall back to one complete release', async () => {
  for (const incomplete of [
    assets.slice(0, -1),
    assets.map((asset, index) => index ? asset : {...asset, size: 0}),
    assets.map((asset, index) => index ? asset : {...asset, state: 'new'}),
  ]) {
    expectRelease(await render({releases: [
      {...release, tag_name: '1.2.0', assets: incomplete}, release,
    ]}), release.tag_name);
  }
});

test('drafts, prereleases, and development tags do not replace stable downloads', async () => {
  expectRelease(await render({releases: [
    {...release, tag_name: '2.0-draft', draft: true},
    {...release, tag_name: '2.0-beta', prerelease: true},
    {...release, tag_name: 'development'},
    {...release, tag_name: 'nightly'},
    release,
  ]}), release.tag_name);
});

test('version text is inserted safely and encoded in download paths', async () => {
  const version = 'v1/<example>';
  expectRelease(await render({releases: [{...release, tag_name: version}]}), version);
});

test('API failures preserve working latest-release and checksum URLs', async () => {
  for (const releases of [new Error('offline'), {ok: false}]) {
    const result = await render({releases});
    for (const control of result.controls) {
      const latest = `${repository}/releases/latest/download/${control.dataset.package}`;
      assert.equal(control.elements['.download-button'].href, latest);
      assert.equal(control.elements['.sha256'].href, `${latest}.sha256sum.txt`);
      assert.equal(control.elements['.sha512'].href, `${latest}.sha512sum.txt`);
      assert.equal(control.elements['.platform-checksums'].hidden, false);
    }
    assert.equal(result.elements['release-status'].hidden, false);
  }
});

test('no complete release sends users to the release list instead of missing files', async () => {
  for (const releases of [[], [{...release, assets: assets.slice(0, -1)}]]) {
    const result = await render({releases});
    for (const control of result.controls) {
      assert.equal(control.elements['.download-button'].href, `${repository}/releases`);
      assert.equal(control.elements['.platform-checksums'].hidden, true);
    }
    assert.equal(result.elements['release-status'].hidden, false);
  }
});

test('development downloads point to matching artifacts from a complete successful main build', async () => {
  const result = await render({runs: [run], builds: {[run.id]: artifacts}});
  expectRelease(result, release.tag_name);
  for (const [index, control] of result.controls.entries()) {
    assert.equal(control.elements['.development-download'].href,
      `${repository}/actions/runs/${run.id}/artifacts/${artifacts[index].id}`);
    assert.match(control.elements['.development-download'].title, /sign-in required/);
  }
});

test('expired or incomplete artifacts use the next complete build without mixing runs', async () => {
  for (const incomplete of [
    artifacts.slice(0, -1),
    artifacts.map((artifact, index) => index ? artifact : {...artifact, expired: true}),
  ]) {
    const older = {...run, id: 122};
    const result = await render({runs: [run, older], builds: {[run.id]: incomplete, [older.id]: artifacts}});
    for (const [index, control] of result.controls.entries()) {
      assert.equal(control.elements['.development-download'].href,
        `${repository}/actions/runs/${older.id}/artifacts/${artifacts[index].id}`);
    }
  }
});

test('failed development lookups retain Actions links and do not disturb stable downloads', async () => {
  for (const runs of [new Error('offline'), {ok: false}, [], [{...run, head_branch: 'other'}]]) {
    const result = await render({runs});
    expectRelease(result, release.tag_name);
    for (const control of result.controls) assert.equal(control.elements['.development-download'].href, workflow);
    assert.match(result.elements['development-status'].textContent, /sign-in/);
  }
});
