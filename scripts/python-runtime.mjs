export const PYTHON_VERSION = "3.12.16";
export const PYTHON_RELEASE = "20261110";

const ASSETS = {
  "aarch64-apple-darwin": {
    archive: "cpython-3.12.16+20261110-aarch64-apple-darwin-install_only_stripped.tar.gz",
    program: "bin/python3",
    sha256: "2f11b8aa3726bdbd87dd9eb76568412faddf4f47e26913fa45d6a2e424972acd",
  },
  "x86_64-apple-darwin": {
    archive: "cpython-3.12.16+20261110-x86_64-apple-darwin-install_only_stripped.tar.gz",
    program: "bin/python3",
    sha256: "b78a4c479f1964ec484880bee567568266f68da102269fb93a05fe8716b45f91",
  },
  "x86_64-pc-windows-msvc": {
    archive: "cpython-3.12.16+20261110-x86_64-pc-windows-msvc-install_only_stripped.tar.gz",
    program: "python.exe",
    sha256: "355ee61b8207b91574491b1c6dc9b0ebb4408fc56bfafe498f3ef439314b52dc",
  },
};

export function pythonAsset(triple) {
  const asset = ASSETS[triple];
  if (!asset) {
    throw new Error(`No Python runtime is listed for ${triple}.`);
  }
  return {
    ...asset,
    url: `https://github.com/astral-sh/python-build-standalone/releases/download/${PYTHON_RELEASE}/${asset.archive}`,
  };
}
