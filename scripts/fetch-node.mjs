import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { access, chmod, copyFile, mkdir, mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { NODE_VERSION, nodeAsset } from "./node-runtime.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binaries = path.join(root, "src-tauri", "binaries");

function hostTriple() {
  const printed = spawnSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" });
  if (printed.status === 0 && printed.stdout.trim()) {
    return printed.stdout.trim();
  }
  const verbose = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = /host: (\S+)/.exec(verbose.stdout || "");
  if (match) {
    return match[1];
  }
  throw new Error("rustc could not report the build target.");
}

function inside(dir, relative) {
  return path.join(dir, ...relative.split("/"));
}

async function download(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Could not download ${url} (${response.status}).`);
  }
  return Buffer.from(await response.arrayBuffer());
}

async function main() {
  const triple = hostTriple();
  const asset = nodeAsset(triple);
  const dest = path.join(binaries, asset.sidecar);
  const stampPath = path.join(binaries, "node.stamp");
  const stamp = `${NODE_VERSION} ${asset.sha256}\n`;
  try {
    if ((await readFile(stampPath, "utf8")) === stamp) {
      await access(dest);
      await access(path.join(binaries, "LICENSE"));
      return;
    }
  } catch {
    // The runtime is missing or was built for another Node version.
  }

  process.stderr.write(`Downloading Node ${NODE_VERSION} for ${triple}.\n`);
  const archive = await download(asset.url);
  const sum = createHash("sha256").update(archive).digest("hex");
  if (sum !== asset.sha256) {
    throw new Error(`The Node download checksum did not match. Expected ${asset.sha256}, got ${sum}.`);
  }

  await mkdir(binaries, { recursive: true });
  const dir = await mkdtemp(path.join(tmpdir(), "buho-node-"));
  const archivePath = path.join(dir, asset.archive);
  const staging = `${dest}.download`;
  try {
    await writeFile(archivePath, archive);
    const extracted = spawnSync("tar", ["-xf", archivePath, "-C", dir], { encoding: "utf8" });
    if (extracted.status !== 0) {
      throw new Error((extracted.stderr || "").trim() || "Could not unpack the Node runtime.");
    }
    await copyFile(inside(dir, asset.binary), staging);
    await chmod(staging, 0o755);
    if (process.platform === "darwin") {
      spawnSync("xattr", ["-c", staging]);
    }
    await rm(dest, { force: true });
    await rename(staging, dest);
    const licenseRoot = asset.binary.split("/")[0];
    await copyFile(path.join(dir, licenseRoot, "LICENSE"), path.join(binaries, "LICENSE"));
    await writeFile(stampPath, stamp);
  } catch (error) {
    await rm(staging, { force: true });
    if (error && error.code === "ENOENT") {
      throw new Error(`The Node archive did not contain ${asset.binary}.`);
    }
    throw error;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message || error}\n`);
  process.exit(1);
});
