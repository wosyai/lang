# Installation

No release installer is published yet. Once the assets and these scripts are on `main`, download the installer from GitHub and run it locally. The script verifies the archive's SHA-256 before replacing a binary. It installs only Wosy, without admin access or shell profile edits.

Linux x64 or macOS arm64:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/wosyai/lang/main/install.sh
sh install.sh
```

Default: `~/.local/bin/wosy`. Add `$HOME/.local/bin` to `PATH` if `wosy` is not found.

Windows x64 (PowerShell):

```powershell
Invoke-WebRequest -UseBasicParsing https://raw.githubusercontent.com/wosyai/lang/main/install.ps1 -OutFile install.ps1
.\install.ps1
```

Default: `%LOCALAPPDATA%\Wosy\bin\wosy.exe`. Add that `bin` directory to your user `PATH` if needed. Pass a release tag as `sh install.sh vX.Y.Z` or `.\install.ps1 -Version vX.Y.Z` to pin the download.

## Additional tools

| Task | Install only if needed |
| --- | --- |
| Parse or check without Git dependencies | Nothing else |
| Fetch or sync Git dependencies | Git |
| Build native programs | Clang, system linker and development SDK |
| Build WASM | Clang and `wasm-ld` |
| Run WASI locally | Wasmtime |
| Build Wosy itself from source | Rust and LLVM 18.1 development libraries |

On Ubuntu 24.04, install native tools with `sudo apt install clang build-essential`. Add `git` for Git dependencies or `lld` for `wasm-ld`. On macOS, `xcode-select --install` installs the system Clang, linker, SDK and Git. For Windows, use LLVM Clang and the Visual Studio C++ Build Tools with a Windows SDK. Windows and macOS are not yet validated. Python is not required by the default Linux scaffold.

## Release assets

Create releases manually from a release tag on `main`, using the platform binaries that were already tested. Keep the release as a draft until all assets are uploaded. Do not trigger validations from the release.

- `wosy-linux-x64.tar.gz` and `wosy-macos-arm64.tar.gz`: one top-level file, `wosy`.
- `wosy-windows-x64.zip`: one top-level file, `wosy.exe`.
- Upload `<archive>.sha256` alongside each archive. Write only the 64-digit SHA-256 digest, followed by a newline.

With the release tag and all six assets ready, run `gh release create <tag> <assets...> --draft --verify-tag`. Publish the draft only after verifying the asset names and hashes.
