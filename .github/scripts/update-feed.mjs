// Merges the per-platform updater entries into latest.json, the manifest the
// Skerry app downloads to find new versions.
//
// Usage: node update-feed.mjs <dir with <slug>.json files> <output file>
// Env:   VERSION, SEMVER, GITHUB_REPOSITORY

import fs from "node:fs";
import path from "node:path";

const [dir, out] = process.argv.slice(2);
const { VERSION, SEMVER, GITHUB_REPOSITORY } = process.env;

const platforms = {};
for (const f of fs.readdirSync(dir).filter((f) => f.endsWith(".json"))) {
  Object.assign(platforms, JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")));
}
for (const os of ["windows", "darwin", "linux"]) {
  if (!Object.keys(platforms).some((k) => k.startsWith(`${os}-`))) throw new Error(`no signed update for ${os}`);
}

const feed = {
  version: SEMVER,
  notes: `Skerry ${VERSION}. Release notes: https://github.com/${GITHUB_REPOSITORY}/releases`,
  pub_date: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
  platforms,
};
fs.writeFileSync(out, JSON.stringify(feed, null, 2) + "\n");
console.log(fs.readFileSync(out, "utf8"));
