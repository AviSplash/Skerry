// Gathers one platform's installers into dist/ (the files the GitHub release
// publishes) and, when the build signed them for the in-app updater, writes
// updater/<slug>.json: the entries for that platform in latest.json.
//
// Usage: node collect-release.mjs <windows|macos|linux>
// Env:   VERSION (release name, e.g. v1.1), SEMVER (e.g. 1.1.0), GITHUB_REPOSITORY

import fs from "node:fs";
import path from "node:path";

const slug = process.argv[2];
const { VERSION, SEMVER, GITHUB_REPOSITORY } = process.env;
const base = `https://github.com/${GITHUB_REPOSITORY}/releases/download/${VERSION}-${slug}`;

const list = (dir, suffix) =>
  fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => f.endsWith(suffix)).map((f) => path.join(dir, f)) : [];

// [file, updater targets it serves, name to publish it under]. The updater
// looks up "<os>-<arch>-<installer>" first, then "<os>-<arch>". Each Linux
// package only serves its own installer type, so a .deb install never gets
// an AppImage.
const mac = "target/universal-apple-darwin/release/bundle";
const files = {
  windows: [
    ...list("target/release/bundle/nsis", "-setup.exe").map((f) => [f, ["windows-x86_64-nsis", "windows-x86_64"]]),
    ...list("target/release/bundle/msi", ".msi").map((f) => [f, ["windows-x86_64-msi"]]),
  ],
  macos: [
    ...list(`${mac}/dmg`, ".dmg").map((f) => [f, []]),
    ...list(`${mac}/macos`, ".app.tar.gz").map((f) => [
      f,
      ["darwin-aarch64-app", "darwin-x86_64-app", "darwin-aarch64", "darwin-x86_64"],
      `Skerry_${SEMVER}_universal.app.tar.gz`,
    ]),
  ],
  linux: [
    ...list("target/release/bundle/appimage", ".AppImage").map((f) => [f, ["linux-x86_64-appimage"]]),
    ...list("target/release/bundle/deb", ".deb").map((f) => [f, ["linux-x86_64-deb"]]),
    ...list("target/release/bundle/rpm", ".rpm").map((f) => [f, ["linux-x86_64-rpm"]]),
  ],
}[slug];
if (!files) throw new Error(`unknown platform ${slug}`);

fs.mkdirSync("dist", { recursive: true });
const platforms = {};
for (const [file, targets, rename] of files) {
  const name = rename ?? path.basename(file);
  // Skip leftovers from other versions (for example from a cached target dir).
  if (!rename && !name.includes(SEMVER)) {
    console.log(`skipping ${file} (not version ${SEMVER})`);
    continue;
  }
  const sig = `${file}.sig`;
  if (!fs.existsSync(sig)) {
    // Unsigned updater bundles are only useful to the updater: skip them.
    if (rename) continue;
  } else {
    for (const t of targets) {
      platforms[t] = { signature: fs.readFileSync(sig, "utf8").trim(), url: `${base}/${encodeURIComponent(name)}` };
    }
  }
  fs.copyFileSync(file, path.join("dist", name));
  console.log(`dist/${name}${fs.existsSync(sig) ? " (signed for the updater)" : ""}`);
}

if (Object.keys(platforms).length) {
  fs.mkdirSync("updater", { recursive: true });
  fs.writeFileSync(path.join("updater", `${slug}.json`), JSON.stringify(platforms, null, 2));
  console.log(`updater/${slug}.json: ${Object.keys(platforms).join(", ")}`);
}
if (process.env.GITHUB_OUTPUT) {
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `signed=${Object.keys(platforms).length > 0}\n`);
}
