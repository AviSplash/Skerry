// Makes sure installed copies of Skerry can download and verify each release.
//
// The in-app updater only installs files signed with the private key whose
// public half is built into the app (plugins.updater.pubkey in
// tauri.conf.json). A release signed with any other key, or with no key, can
// never be installed by the updater. So:
//
//   node updater-key.mjs prepare <tauri.conf.json>
//       Before building. Requires TAURI_SIGNING_PRIVATE_KEY (and its
//       password). If TAURI_UPDATER_PUBLIC_KEY is set, builds with that
//       public key. Then signs a probe file with the private key and checks
//       it against the public key the app will be built with: a mismatch
//       fails here, before anything is built. In GitHub Actions it then hands
//       the build the key as a file, with stray whitespace around the pasted
//       secrets removed (a trailing newline makes the key unreadable).
//
//   node updater-key.mjs verify-feed <latest.json> <tauri.conf.json>
//       Before publishing the feed. Every kind of installation must have an
//       entry; downloads every file it points to and checks its signature,
//       exactly as the app will (SEMVER must match).
//
//   node updater-key.mjs self-test
//       Checks this script against a throwaway key pair made by the Tauri
//       CLI (used in CI).
//
// Signatures are minisign signatures, as produced by `tauri signer`.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const TAURI_CLI = "@tauri-apps/cli@2";
/** Every kind of installation Skerry ships; each must be able to update. */
const TARGETS = [
  "windows-x86_64-nsis",
  "windows-x86_64-msi",
  "darwin-aarch64-app",
  "darwin-x86_64-app",
  "linux-x86_64-appimage",
  "linux-x86_64-deb",
  "linux-x86_64-rpm",
];
const SETUP = "Run .github/scripts/make-updater-key.sh once and add the secrets it prints (see docs/releasing.md).";

/** The text inside a base64-encoded minisign key or signature file. */
function unbox(b64) {
  return Buffer.from(String(b64).trim(), "base64").toString("utf8");
}

function keyIdHex(id) {
  return Buffer.from(id).reverse().toString("hex").toUpperCase();
}

/** Parse the contents of a `.pub` file (base64, as Tauri writes it). */
export function parsePublicKey(b64) {
  const lines = unbox(b64)
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  const raw = Buffer.from(lines[1] ?? "", "base64");
  if (raw.length !== 42 || raw.subarray(0, 2).toString() !== "Ed") {
    throw new Error("that isn't a minisign public key");
  }
  const key = crypto.createPublicKey({
    key: { kty: "OKP", crv: "Ed25519", x: raw.subarray(10).toString("base64url") },
    format: "jwk",
  });
  return { id: raw.subarray(2, 10), key };
}

/** Throw unless `sig` (the contents of a `.sig` file) signs `data` with `pub`. */
export function verify(data, sig, pub) {
  const lines = unbox(sig).split("\n");
  const blob = Buffer.from((lines[1] ?? "").trim(), "base64");
  if (blob.length !== 74) throw new Error("the signature is malformed");
  const alg = blob.subarray(0, 2).toString();
  const id = blob.subarray(2, 10);
  const signature = blob.subarray(10);
  if (!id.equals(pub.id)) {
    throw new Error(`it's signed with key ${keyIdHex(id)}, but Skerry trusts key ${keyIdHex(pub.id)}`);
  }
  let message;
  if (alg === "ED") message = crypto.createHash("blake2b512").update(data).digest();
  else if (alg === "Ed") message = data;
  else throw new Error(`unknown signature algorithm ${JSON.stringify(alg)}`);
  if (!crypto.verify(null, message, pub.key, signature)) throw new Error("the signature doesn't match the file");
  const prefix = "trusted comment: ";
  if (!(lines[2] ?? "").startsWith(prefix)) throw new Error("the signature has no trusted comment");
  const trusted = Buffer.from(lines[2].slice(prefix.length));
  const global = Buffer.from((lines[3] ?? "").trim(), "base64");
  if (!crypto.verify(null, Buffer.concat([signature, trusted]), pub.key, global)) {
    throw new Error("the signature's trusted comment was altered");
  }
}

function fail(message) {
  console.log(`::error::${message}`);
  process.exit(1);
}

function readConfig(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** Sign `file` with the Tauri CLI (TAURI_SIGNING_PRIVATE_KEY[_PASSWORD] from the environment). */
function tauriSign(file, env = process.env) {
  const r = spawnSync("npx", ["--yes", TAURI_CLI, "signer", "sign", file], {
    env,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  if (r.status !== 0) {
    const out = (r.stderr || r.stdout || "").trim().split("\n");
    throw new Error(out[out.length - 1].trim().replace(/^Error\s+/, "") || "tauri signer sign failed");
  }
  return fs.readFileSync(`${file}.sig`, "utf8");
}

function prepare(configFile) {
  const privateKey = (process.env.TAURI_SIGNING_PRIVATE_KEY || "").trim();
  const password = (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD || "").trim();
  if (!privateKey) {
    fail(`No TAURI_SIGNING_PRIVATE_KEY secret: installed copies of Skerry couldn't update to this release. ${SETUP}`);
  }
  const config = readConfig(configFile);
  const fromSecret = (process.env.TAURI_UPDATER_PUBLIC_KEY || "").trim();
  if (fromSecret) config.plugins.updater.pubkey = fromSecret;
  config.bundle.createUpdaterArtifacts = true;

  let pub;
  try {
    pub = parsePublicKey(config.plugins.updater.pubkey);
  } catch (e) {
    fail(`The updater public key is unusable (${e.message}). ${SETUP}`);
  }

  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "skerry-updater-"));
  const probe = path.join(dir, "probe.bin");
  fs.writeFileSync(probe, crypto.randomBytes(64));
  try {
    let sig;
    try {
      sig = tauriSign(probe, {
        ...process.env,
        TAURI_SIGNING_PRIVATE_KEY: privateKey,
        TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password,
      });
    } catch (e) {
      fail(`Couldn't sign with TAURI_SIGNING_PRIVATE_KEY (${e.message}). Check TAURI_SIGNING_PRIVATE_KEY_PASSWORD.`);
    }
    try {
      verify(fs.readFileSync(probe), sig, pub);
    } catch (e) {
      fail(
        `TAURI_SIGNING_PRIVATE_KEY doesn't belong to the public key Skerry would be built with: ${e.message}. ` +
          (fromSecret
            ? "Check that TAURI_UPDATER_PUBLIC_KEY is the .pub file made together with this private key."
            : "Add the matching public key as the TAURI_UPDATER_PUBLIC_KEY secret."),
      );
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  fs.writeFileSync(configFile, JSON.stringify(config, null, 2) + "\n");

  // Later steps (the Tauri build) sign with the cleaned-up key, as a file.
  if (process.env.GITHUB_ENV) {
    const keyFile = path.join(process.env.RUNNER_TEMP || os.tmpdir(), "skerry-updater.key");
    fs.writeFileSync(keyFile, privateKey, { mode: 0o600 });
    if (password) console.log(`::add-mask::${password}`);
    fs.appendFileSync(
      process.env.GITHUB_ENV,
      `TAURI_SIGNING_PRIVATE_KEY=${keyFile}\nTAURI_SIGNING_PRIVATE_KEY_PASSWORD=${password}\n`,
    );
  }
  console.log(`Updates will be signed with key ${keyIdHex(pub.id)}, which this build trusts.`);
}

async function verifyFeed(feedFile, configFile) {
  const feed = JSON.parse(fs.readFileSync(feedFile, "utf8"));
  const pubkey = (process.env.TAURI_UPDATER_PUBLIC_KEY || "").trim() || readConfig(configFile).plugins.updater.pubkey;
  const pub = parsePublicKey(pubkey);
  if (process.env.SEMVER && feed.version !== process.env.SEMVER) {
    fail(`latest.json says version ${feed.version}, but this release is ${process.env.SEMVER}.`);
  }
  const missing = TARGETS.filter((t) => !feed.platforms[t]);
  if (missing.length) fail(`latest.json has no update for ${missing.join(", ")}: those installations couldn't update.`);
  const checked = new Map();
  for (const [target, { url, signature }] of Object.entries(feed.platforms)) {
    const key = `${url}\n${signature}`;
    if (!checked.has(key)) {
      const res = await fetch(url, { redirect: "follow" });
      if (!res.ok) fail(`${target}: downloading ${url} failed (HTTP ${res.status}).`);
      const data = Buffer.from(await res.arrayBuffer());
      try {
        verify(data, signature, pub);
      } catch (e) {
        fail(`${target}: ${url} can't be installed by the updater: ${e.message}.`);
      }
      checked.set(key, data.length);
    }
    console.log(`${target}: ${url} (${checked.get(key)} bytes) verified with key ${keyIdHex(pub.id)}`);
  }
}

function selfTest() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "skerry-updater-test-"));
  try {
    const keyFile = path.join(dir, "test.key");
    const r = spawnSync("npx", ["--yes", TAURI_CLI, "signer", "generate", "--ci", "-p", "test", "-w", keyFile], {
      encoding: "utf8",
      shell: process.platform === "win32",
    });
    if (r.status !== 0) throw new Error(`tauri signer generate failed: ${r.stderr}`);
    const pub = parsePublicKey(fs.readFileSync(`${keyFile}.pub`, "utf8"));
    const env = {
      ...process.env,
      TAURI_SIGNING_PRIVATE_KEY: fs.readFileSync(keyFile, "utf8"),
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "test",
    };
    const file = path.join(dir, "update.bin");
    fs.writeFileSync(file, crypto.randomBytes(4096));
    const sig = tauriSign(file, env);
    verify(fs.readFileSync(file), sig, pub);

    const expectFailure = (what, f) => {
      try {
        f();
      } catch {
        return;
      }
      throw new Error(`${what} was accepted`);
    };
    const tampered = fs.readFileSync(file);
    tampered[100] ^= 1;
    expectFailure("a changed file", () => verify(tampered, sig, pub));
    const otherKey = parsePublicKey(readConfig("apps/skerry-app/tauri.conf.json").plugins.updater.pubkey);
    expectFailure("another key", () => verify(fs.readFileSync(file), sig, otherKey));
    const lines = unbox(sig).split("\n");
    lines[2] = lines[2].replace("file:", "file:x");
    const forged = Buffer.from(lines.join("\n")).toString("base64");
    expectFailure("a changed trusted comment", () => verify(fs.readFileSync(file), forged, pub));
    console.log("updater signature checks work");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const [command, ...args] = process.argv.slice(2);
if (command === "prepare") prepare(args[0]);
else if (command === "verify-feed") await verifyFeed(args[0], args[1]);
else if (command === "self-test") selfTest();
else fail(`usage: node updater-key.mjs prepare|verify-feed|self-test`);
