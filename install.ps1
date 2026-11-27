#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidatePattern('\Av[0-9]+\.[0-9]+\.[0-9]+\z')]
    [string] $Version,
    [string] $InstallDir
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT' -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This installer supports Windows x64 only.'
}

if (-not $InstallDir) {
    $InstallDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Wosy\bin'
}
$InstallDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallDir)
$destination = Join-Path $InstallDir 'wosy.exe'

$asset = 'wosy-windows-x64.zip'
$releases = 'https://github.com/wosyai/lang/releases'
$download = if ($Version) { "$releases/download/$Version" } else { "$releases/latest/download" }
$scratch = Join-Path ([IO.Path]::GetTempPath()) ("wosy-install-" + [Guid]::NewGuid().ToString('N'))
$stage = $null
try {
    [void] [IO.Directory]::CreateDirectory($scratch)
    $zip = Join-Path $scratch $asset
    $checksum = "$zip.sha256"
    Invoke-WebRequest -UseBasicParsing -Uri "$download/$asset.sha256" -OutFile $checksum
    $line = [IO.File]::ReadAllText($checksum).Trim()
    $pattern = '\A(?<hash>[0-9a-fA-F]{64})(?:[ \t]+\*?' + [regex]::Escape($asset) + ')?\z'
    $digest = [regex]::Match($line, $pattern)
    if (-not $digest.Success) {
        throw "Invalid checksum for $asset."
    }
    $expected = $digest.Groups['hash'].Value

    Invoke-WebRequest -UseBasicParsing -Uri "$download/$asset" -OutFile $zip
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $expected) {
        throw "SHA256 mismatch for $asset."
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        if ($archive.Entries.Count -ne 1 -or $archive.Entries[0].FullName -cne 'wosy.exe') {
            throw "Expected $asset to contain only wosy.exe at its root."
        }
        # Stage on the destination volume; extraction never uses a ZIP-supplied path.
        [void] [IO.Directory]::CreateDirectory($InstallDir)
        $stage = Join-Path $InstallDir ('.wosy-install-' + [Guid]::NewGuid().ToString('N'))
        [void] [IO.Directory]::CreateDirectory($stage)
        $candidate = Join-Path $stage 'wosy.exe'
        [IO.Compression.ZipFileExtensions]::ExtractToFile($archive.Entries[0], $candidate)
    } finally {
        $archive.Dispose()
    }

    if ([IO.File]::Exists($destination)) {
        $backup = Join-Path $stage 'previous.exe'
        try {
            [IO.File]::Replace($candidate, $destination, $backup)
        } catch {
            if ([IO.File]::Exists($backup)) {
                # Keep any existing backup on failure, including a partial OS replacement.
                $stage = $null
                Write-Warning "The previous binary is backed up at $backup"
            }
            throw
        }
    } else {
        [IO.File]::Move($candidate, $destination)
    }
} finally {
    if ($stage) { Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Installed Wosy to $destination"
$quoted = $destination.Replace("'", "''")
Write-Host "Try: & '$quoted' --help"
Write-Host "To use wosy by name, add this folder to your user PATH: $InstallDir"
