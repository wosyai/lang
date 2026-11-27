# Installation

These installers require published release assets on GitHub. This branch does not publish a release. The scripts verify SHA-256 before replacing the binary; neither edits PATH nor installs additional tools.

Linux x64 or macOS arm64:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/wosyai/lang/main/install.sh
sh install.sh
```

Default: `~/.local/bin/wosy`. Add that directory to PATH if needed.

Windows x64 (PowerShell 5.1 or later):

```powershell
Invoke-WebRequest -UseBasicParsing https://raw.githubusercontent.com/wosyai/lang/main/install.ps1 -OutFile install.ps1
.\install.ps1
```

Default: `%LOCALAPPDATA%\Wosy\bin\wosy.exe`. Add that directory to PATH if needed. To pin a release, pass its tag to `sh install.sh vX.Y.Z` or `.\install.ps1 -Version vX.Y.Z`.

## Toolchain

- Parse without Git dependencies: Wosy only.
- Fetch and sync Git dependencies on supported Linux hosts: Git.
- Use the current Linux reference scaffold scripts: Python 3.
- Build native programs: Clang, a system linker and SDK, plus what the project's builder requires.
- Build WASM: the project's WASM builder, typically Clang and `wasm-ld`.
- Run WASI: Wasmtime if configured as the runner.
- Build Wosy from source: Rust and LLVM 18.1 development libraries.

The project runner receives the artifact manifest, a literal `--`, then the program arguments. A Windows native LLVM build records `main.exe`; use the path from the manifest. Other artifact names are unchanged.

Native validation of this revision on Windows and macOS is pending. `wosy init` and the Git dependency workflow remain Linux-only.

## Release assets

- `wosy-linux-x64.tar.gz` and `wosy-macos-arm64.tar.gz`: one root file, `wosy`.
- `wosy-windows-x64.zip`: one root file, `wosy.exe`.
- Each archive gets a separate `.sha256` file containing its 64-digit digest and a newline.
