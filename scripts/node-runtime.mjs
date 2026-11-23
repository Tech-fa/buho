export const NODE_VERSION = "22.23.5";

const ASSETS = {
  "aarch64-apple-darwin": {
    archive: "node-v22.23.5-darwin-arm64.tar.gz",
    binary: "node-v22.23.5-darwin-arm64/bin/node",
    sha256: "e7131995759b61a0170b646010d2ad35a5d37b4cb0d471f3e8317eac6f742f3d",
  },
  "x86_64-apple-darwin": {
    archive: "node-v22.23.5-darwin-x64.tar.gz",
    binary: "node-v22.23.5-darwin-x64/bin/node",
    sha256: "c53383238206b9ae4db23c5d8a63645e2e753bec8ab51a49cc81939aa64e9422",
  },
  "x86_64-pc-windows-msvc": {
    archive: "node-v22.23.5-win-x64.zip",
    binary: "node-v22.23.5-win-x64/node.exe",
    sha256: "4a191082e9706866830379d577d2cc8a50c776fcd9436a8d061b063ed0d9a0e3",
  },
};

export function nodeAsset(triple) {
  const asset = ASSETS[triple];
  if (!asset) {
    throw new Error(`No Node runtime is listed for ${triple}.`);
  }
  const extension = triple.includes("windows") ? ".exe" : "";
  return {
    ...asset,
    url: `https://nodejs.org/dist/v${NODE_VERSION}/${asset.archive}`,
    sidecar: `node-${triple}${extension}`,
  };
}
