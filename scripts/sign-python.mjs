import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pythonDir = path.join(root, "src-tauri", "binaries", "python");
const entitlements = path.join(root, "src-tauri", "entitlements.plist");

const certificate = process.env.APPLE_CERTIFICATE || "";
const certificatePassword = process.env.APPLE_CERTIFICATE_PASSWORD || "";
if (process.platform !== "darwin" || !certificate || !certificatePassword) {
  process.exit(0);
}

function run(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.status !== 0) {
    const detail = (result.stderr || result.stdout || `${command} failed`).trim();
    throw new Error(detail);
  }
  return result.stdout || "";
}

function machOFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isSymbolicLink()) {
      continue;
    }
    if (entry.isDirectory()) {
      found.push(...machOFiles(full));
      continue;
    }
    if (!entry.isFile()) {
      continue;
    }
    const info = spawnSync("file", ["-b", full], { encoding: "utf8" });
    if ((info.stdout || "").includes("Mach-O")) {
      found.push(full);
    }
  }
  return found;
}

function keychainList() {
  const listed = run("security", ["list-keychains", "-d", "user"]);
  return listed
    .split("\n")
    .map((line) => line.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

const work = mkdtempSync(path.join(tmpdir(), "buho-python-sign-"));
const keychain = path.join(work, "signing.keychain-db");
const keychainPassword = randomBytes(24).toString("hex");
const certificatePath = path.join(work, "certificate.p12");
const previousKeychains = keychainList();
const previousDefault = run("security", ["default-keychain"])
  .trim()
  .replace(/^"|"$/g, "");
let keychainReady = false;
try {
  writeFileSync(certificatePath, Buffer.from(certificate, "base64"));
  run("security", ["create-keychain", "-p", keychainPassword, keychain]);
  keychainReady = true;
  run("security", ["set-keychain-settings", "-lut", "21600", keychain]);
  run("security", ["unlock-keychain", "-p", keychainPassword, keychain]);
  // codesign does not look in a keychain unless it is on the search list.
  // --keychain alone still reports "no identity found" on the GitHub runner.
  run("security", ["list-keychains", "-d", "user", "-s", keychain, ...previousKeychains]);
  run("security", ["default-keychain", "-s", keychain]);
  run("security", [
    "import",
    certificatePath,
    "-k",
    keychain,
    "-P",
    certificatePassword,
    "-T",
    "/usr/bin/codesign",
    "-f",
    "pkcs12",
  ]);
  run("security", [
    "set-key-partition-list",
    "-S",
    "apple-tool:,apple:,codesign:",
    "-s",
    "-k",
    keychainPassword,
    keychain,
  ]);
  const listed = run("security", ["find-identity", "-v", "-p", "codesigning", keychain]);
  const identity = listed
    .split("\n")
    .map((line) => /"([^"]+)"/.exec(line)?.[1] || "")
    .find((name) => name.includes("Developer ID Application"));
  if (!identity) {
    throw new Error("The Apple certificate has no Developer ID Application identity.");
  }
  const requested = process.env.APPLE_SIGNING_IDENTITY || "";
  if (requested && !identity.includes(requested) && !requested.includes(identity)) {
    throw new Error(`The certificate is ${identity}, which does not match APPLE_SIGNING_IDENTITY.`);
  }
  const files = machOFiles(pythonDir);
  if (files.length === 0) {
    throw new Error("The Python runtime has no executable files to sign.");
  }
  for (const file of files) {
    run("codesign", [
      "--force",
      "--sign",
      identity,
      "--keychain",
      keychain,
      "--options",
      "runtime",
      "--timestamp",
      "--entitlements",
      entitlements,
      file,
    ]);
  }
} catch (error) {
  process.stderr.write(`${error.message || error}\n`);
  process.exitCode = 1;
} finally {
  if (previousKeychains.length > 0) {
    spawnSync("security", ["list-keychains", "-d", "user", "-s", ...previousKeychains]);
  }
  if (previousDefault) {
    spawnSync("security", ["default-keychain", "-s", previousDefault]);
  }
  if (keychainReady) {
    spawnSync("security", ["delete-keychain", keychain]);
  }
  rmSync(work, { recursive: true, force: true });
}
