import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { access, chmod, mkdir, mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PYTHON_VERSION, pythonAsset } from "./python-runtime.mjs";

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
  const asset = pythonAsset(triple);
  const dest = path.join(binaries, "python");
  const program = inside(dest, asset.program);
  const stampPath = path.join(binaries, "python.stamp");
  const stamp = `${PYTHON_VERSION} ${asset.sha256}\n`;
  try {
    if ((await readFile(stampPath, "utf8")) === stamp) {
      await access(program);
      return;
    }
  } catch {
    // The runtime is missing or was built for another Python version.
  }

  process.stderr.write(`Downloading Python ${PYTHON_VERSION} for ${triple}.\n`);
  const archive = await download(asset.url);
  const sum = createHash("sha256").update(archive).digest("hex");
  if (sum !== asset.sha256) {
    throw new Error(`The Python download checksum did not match. Expected ${asset.sha256}, got ${sum}.`);
  }

  await mkdir(binaries, { recursive: true });
  const staging = path.join(binaries, "python.download");
  await rm(staging, { recursive: true, force: true });
  await mkdir(staging);
  const archivePath = path.join(staging, asset.archive);
  try {
    await writeFile(archivePath, archive);
    const extracted = spawnSync("tar", ["-xf", archivePath, "-C", staging], { encoding: "utf8" });
    if (extracted.status !== 0) {
      throw new Error((extracted.stderr || "").trim() || "Could not unpack the Python runtime.");
    }
    const unpacked = path.join(staging, "python");
    await access(inside(unpacked, asset.program));
    await rm(dest, { recursive: true, force: true });
    await rename(unpacked, dest);
    await chmod(program, 0o755);
    if (process.platform === "darwin") {
      spawnSync("xattr", ["-cr", dest]);
    }
    await writeFile(stampPath, stamp);
  } catch (error) {
    await rm(dest, { recursive: true, force: true });
    if (error && error.code === "ENOENT") {
      throw new Error(`The Python archive did not contain python/${asset.program}.`);
    }
    throw error;
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message || error}\n`);
  process.exit(1);
});
