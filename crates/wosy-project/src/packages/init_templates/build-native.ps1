param($Partition, $OutputDirectory, $Manifest, $Runtime)
$ErrorActionPreference = 'Stop'
$Artifact = Get-Content -LiteralPath $Manifest -Raw -Encoding UTF8 | ConvertFrom-Json
& clang.exe -x ir $Partition $Runtime -o $Artifact.executable
exit $LASTEXITCODE
