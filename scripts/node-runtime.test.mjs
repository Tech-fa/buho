import assert from "node:assert/strict";
import test from "node:test";
import { nodeAsset } from "./node-runtime.mjs";

test("an Apple Silicon build ships the arm64 Node binary", () => {
  const asset = nodeAsset("aarch64-apple-darwin");
  assert.equal(asset.archive, "node-v22.23.5-darwin-arm64.tar.gz");
  assert.equal(asset.binary, "node-v22.23.5-darwin-arm64/bin/node");
  assert.equal(asset.sidecar, "node-aarch64-apple-darwin");
  assert.equal(asset.sha256, "e7131995759b61a0170b646010d2ad35a5d37b4cb0d471f3e8317eac6f742f3d");
  assert.equal(asset.url, "https://nodejs.org/dist/v22.23.5/node-v22.23.5-darwin-arm64.tar.gz");
});

test("a Windows build ships node.exe", () => {
  const asset = nodeAsset("x86_64-pc-windows-msvc");
  assert.equal(asset.sidecar, "node-x86_64-pc-windows-msvc.exe");
  assert.equal(asset.binary.endsWith("/node.exe"), true);
});

test("an unknown target is refused", () => {
  assert.throws(() => nodeAsset("sparc-unknown-none"), /No Node runtime is listed/);
});
