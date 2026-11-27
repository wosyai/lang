function Invoke-WosyProgram {
    param([string] $Executable, [string[]] $ProgramArguments)

    # .NET Framework expects a single Windows argument string. Quote each
    # argument, including empty ones, without PowerShell's native re-parsing.
    $CommandLine = New-Object System.Text.StringBuilder
    foreach ($Argument in $ProgramArguments) {
        if ($CommandLine.Length -gt 0) { [void] $CommandLine.Append(' ') }
        [void] $CommandLine.Append('"')
        $Backslashes = 0
        foreach ($Character in $Argument.ToCharArray()) {
            if ($Character -eq '\') {
                $Backslashes++
                continue
            }
            if ($Character -eq '"') {
                [void] $CommandLine.Append('\', ($Backslashes * 2 + 1))
            } else {
                [void] $CommandLine.Append('\', $Backslashes)
            }
            [void] $CommandLine.Append($Character)
            $Backslashes = 0
        }
        [void] $CommandLine.Append('\', ($Backslashes * 2))
        [void] $CommandLine.Append('"')
    }

    $Start = New-Object System.Diagnostics.ProcessStartInfo
    $Start.FileName = $Executable
    $Start.Arguments = $CommandLine.ToString()
    $Start.UseShellExecute = $false
    $Process = [System.Diagnostics.Process]::Start($Start)
    try {
        $Process.WaitForExit()
        return $Process.ExitCode
    } finally {
        $Process.Dispose()
    }
}
