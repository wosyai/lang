#!/bin/sh
set -eu
expected=d524f9c3ffdfa8c0f56414a0385cb60beed79e82
test "$(git rev-parse HEAD)" = "$expected"
git diff --quiet HEAD
test "$(uname -s):$(uname -m)" = Darwin:arm64
: "${LLVM_SYS_181_PREFIX:=$(brew --prefix llvm@18)}"
export LLVM_SYS_181_PREFIX
case "$("$LLVM_SYS_181_PREFIX/bin/llvm-config" --version)" in 18.1.*) ;; *) exit 2 ;; esac
command -v xcrun >/dev/null
xcrun --show-sdk-path >/dev/null
xcodebuild -version
rustc -Vv
"$LLVM_SYS_181_PREFIX/bin/llvm-config" --version
export RUSTC_WRAPPER= CARGO_ENCODED_RUSTFLAGS=
: "${CARGO_TARGET_DIR:=$PWD/target}"
case "$CARGO_TARGET_DIR" in /*) ;; *) CARGO_TARGET_DIR=$PWD/$CARGO_TARGET_DIR ;; esac
export CARGO_TARGET_DIR
export PATH="$LLVM_SYS_181_PREFIX/bin:$PATH"
export SDKROOT="$(xcrun --show-sdk-path)"
cargo build --release --locked -p wosy-cli
wosy="$CARGO_TARGET_DIR/release/wosy"
test "$(/usr/bin/lipo -archs "$wosy")" = arm64
/usr/bin/shasum -a 256 "$wosy"
/usr/bin/otool -L "$wosy"

project="$(mktemp -d "${TMPDIR:-/tmp}/wosy-dev2-mac.XXXXXXXX")"
trap 'rm -rf "$project"' EXIT HUP INT TERM
cd "$project"
mkdir src scripts
cat > wosy.toml <<'TOML'
[package]
name = "native_smoke"
default_target = "app"

[comments]
categories = []

[builders.native]
command = ["/bin/sh", "scripts/build-native"]
args = []
toolchain_identity = "llvm-18.1"

[runners.native]
command = ["/bin/sh", "scripts/run-native"]
args = []

[targets.app]
root = "src/main.w"

[targets.app.artifacts.main]
artifact = "Main"

[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"

[targets.app.profiles.dev]
main_artifact = "main"
TOML
cat > src/main.w <<'WOSY'
%%start
i32(i32) twice = fn(value) { value + value };
if (twice(7) != 14) { core.system_panic(); };
%%end
WOSY
cat > scripts/build-native <<'SH'
#!/bin/sh
set -eu
test "$#" -eq 4
p=$(/usr/bin/plutil -extract executable raw -o - "$3")
test "$p" = "$2/main"
clang -x ir "$1" "$4" -o "$p"
SH
cat > scripts/run-native <<'SH'
#!/bin/sh
set -eu
test "$#" -eq 3
p=$(/usr/bin/plutil -extract executable raw -o - "$1")
test "$2" = -- && test "$3" = 'two words'
exec "$p" "$3"
SH
"$wosy" parse app
"$wosy" build app --profile dev
test ! -e .wosy/artifacts/app/dev/main/artifact-manifest.pending.json
p=$(/usr/bin/plutil -extract executable raw -o - .wosy/artifacts/app/dev/main/artifact-manifest.json)
test "${p##*/}" = main
test "$(/usr/bin/lipo -archs "$p")" = arm64
"$wosy" run app --profile dev -- 'two words'
printf '%s\n' 'Dev2 macOS arm64 native smoke passed'
