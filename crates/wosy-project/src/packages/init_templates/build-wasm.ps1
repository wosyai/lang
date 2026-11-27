param($Partition, $OutputDirectory, $Manifest, $Runtime)
$ErrorActionPreference = 'Stop'
$Artifact = Get-Content -LiteralPath $Manifest -Raw -Encoding UTF8 | ConvertFrom-Json
$MainObject = Join-Path $OutputDirectory 'main.o'
$StartObject = Join-Path $OutputDirectory 'start.o'
$RuntimeObject = Join-Path $OutputDirectory 'runtime.o'
& clang.exe --target=wasm32-wasi -c -x ir $Partition -o $MainObject
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& clang.exe --target=wasm32-wasi -c -x ir scripts/start.ll -o $StartObject
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& clang.exe --target=wasm32-wasi -c -x ir $Runtime -o $RuntimeObject
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& wasm-ld.exe --no-entry --allow-undefined --export=_start $MainObject $StartObject $RuntimeObject -o $Artifact.executable
exit $LASTEXITCODE
