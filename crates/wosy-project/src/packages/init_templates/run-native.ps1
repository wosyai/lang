$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'run-program.ps1')
# Skip the manifest path and -- passed by the CLI.
$Forward = @()
if ($args.Count -gt 2) { $Forward = $args[2..($args.Count - 1)] }
exit (Invoke-WosyProgram -Executable $env:WOSY_ARTIFACT_EXECUTABLE -ProgramArguments $Forward)
