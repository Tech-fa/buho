import assert from "node:assert/strict";
import test from "node:test";
import { pythonAsset } from "./python-runtime.mjs";

test("an Apple Silicon build ships the stripped Python runtime", () => {
  const asset = pythonAsset("aarch64-apple-darwin");
  assert.equal(asset.program, "bin/python3");
  assert.equal(
    asset.archive,
    "cpython-3.12.16+20261110-aarch64-apple-darwin-install_only_stripped.tar.gz",
  );
  assert.equal(asset.sha256, "2f11b8aa3726bdbd87dd9eb76568412faddf4f47e26913fa45d6a2e424972acd");
  assert.equal(
    asset.url,
    "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/cpython-3.12.16+20261110-aarch64-apple-darwin-install_only_stripped.tar.gz",
  );
});

test("a Windows build ships python.exe", () => {
  const asset = pythonAsset("x86_64-pc-windows-msvc");
  assert.equal(asset.program, "python.exe");
  assert.equal(
    asset.archive,
    "cpython-3.12.16+20261110-x86_64-pc-windows-msvc-install_only_stripped.tar.gz",
  );
});

test("an unknown target is refused", () => {
  assert.throws(() => pythonAsset("sparc-unknown-none"), /No Python runtime is listed/);
});
