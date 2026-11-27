$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'run-program.ps1')
$Forward = @('run', $env:WOSY_ARTIFACT_EXECUTABLE)
if ($args.Count -gt 2) { $Forward += $args[2..($args.Count - 1)] }
$Wasmtime = (Get-Command wasmtime.exe -CommandType Application).Path
exit (Invoke-WosyProgram -Executable $Wasmtime -ProgramArguments $Forward)
