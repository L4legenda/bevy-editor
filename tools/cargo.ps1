# Run Cargo with a build lock and artifact inventory. Cleanup is manual only.
# Examples: .\tools\cargo.ps1 build --locked --bin bevy-editor
#           .\tools\cargo.ps1 test --locked
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1's filesystem provider cannot Join-Path a verbatim drive
# returned by Rust canonicalize; use its equivalent ordinary Windows spelling.
function Get-OrdinaryPath([string] $value) {
    if ($value.StartsWith('\\?\UNC\')) { return '\\' + $value.Substring(8) }
    if ($value.StartsWith('\\?\')) { return $value.Substring(4) }
    return $value
}
function Initialize-CacheTag([string] $directory) {
    $tag = Join-Path $directory 'CACHEDIR.TAG'
    if (Test-Path -LiteralPath $tag) {
        if ((Get-Item -LiteralPath $tag -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing to use a linked cache directory tag.'
        }
        return
    }
    $stream = [IO.File]::Open($tag, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes("Signature: 8a477f597d28d172789f06886806bc55`n# Bevy Editor build cache.`n")
        $stream.Write($bytes, 0, $bytes.Length)
    } finally {
        $stream.Dispose()
    }
}
$scriptRoot = Get-OrdinaryPath $PSScriptRoot
$projectRoot = [IO.Path]::GetFullPath([IO.Path]::Combine($scriptRoot, '..'))
$targetPath = Join-Path $projectRoot 'target'
$cargoArgs = @($args)
if ($cargoArgs.Count -eq 0) { $cargoArgs = @('build', '--locked', '--bin', 'bevy-editor') }
if ($cargoArgs[0] -notin @('build', 'check', 'test', 'run', 'clippy', 'clean')) {
    throw 'Supported commands: build, check, test, run, clippy, clean.'
}
if ($cargoArgs[0] -eq 'clean' -and $cargoArgs.Count -ne 1) {
    throw 'Clean accepts no extra arguments and only removes this project target directory.'
}
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargoPath)) { throw 'Cargo was not found. Install Rust first.' }
Push-Location -LiteralPath $projectRoot
$helper = $null
try {
    if (Test-Path -LiteralPath $targetPath) {
        $targetItem = Get-Item -LiteralPath $targetPath -Force
        if ($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing to use a target directory that is a link or junction.'
        }
    }
    # Cover bootstrap with the same launcher lock. The Rust process acquires it
    # itself for the whole Cargo command and inventory update that follow.
    # Manual cleanup runs under this PowerShell lock without bootstrapping a helper.
    $lockPath = Join-Path $projectRoot '.bevy-editor-build.lock'
    if ((Test-Path -LiteralPath $lockPath) -and ((Get-Item -LiteralPath $lockPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Refusing to use a linked build lock.'
    }
    try {
        $buildLock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    } catch {
        throw 'Another editor build is active. Wait for it to finish before starting another.'
    }
    try {
        if ($cargoArgs[0] -eq 'clean') {
            if ($env:OS -eq 'Windows_NT') {
                $targetPrefix = $targetPath.TrimEnd('\') + '\'
                $running = @(Get-CimInstance Win32_Process | Where-Object {
                    $_.ExecutablePath -and (Get-OrdinaryPath $_.ExecutablePath).StartsWith($targetPrefix, [StringComparison]::OrdinalIgnoreCase)
                })
                if ($running.Count -gt 0) {
                    throw 'Close programs running from target before cleaning the build cache.'
                }
            }
            # Older wrappers created target before Cargo, without a CACHEDIR.TAG.
            # Adopt only an existing cache identified by our valid version-1 inventory.
            $indexPath = Join-Path $targetPath '.bevy-editor-cache.json'
            if ((Test-Path -LiteralPath $indexPath -PathType Leaf) -and -not ((Get-Item -LiteralPath $indexPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                $inventory = $null
                try { $inventory = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json } catch {}
                if ($inventory.version -eq 1 -and $null -ne $inventory.snapshots -and $null -ne $inventory.known) {
                    Initialize-CacheTag $targetPath
                }
            }
            & $cargoPath clean --manifest-path (Join-Path $projectRoot 'Cargo.toml') --target-dir $targetPath
            $result = $LASTEXITCODE
        } else {
            New-Item -ItemType Directory -Path $targetPath -Force | Out-Null
            Initialize-CacheTag $targetPath
            $hostLine = (& $cargoPath -vV | Select-String '^host: ').ToString()
            $hostTarget = $hostLine.Substring(6).Trim()
            if (-not $hostTarget) { throw 'Could not determine the Rust host target.' }
            $helperTarget = Join-Path $targetPath '.bevy-editor-tools/cargo-cache'
            foreach ($directory in @((Join-Path $targetPath '.bevy-editor-tools'), $helperTarget)) {
                if ((Test-Path -LiteralPath $directory) -and ((Get-Item -LiteralPath $directory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                    throw 'Refusing to build the cache manager in a linked directory.'
                }
            }
            & $cargoPath build --locked --quiet --manifest-path (Join-Path $scriptRoot 'cargo-cache/Cargo.toml') --target-dir $helperTarget --target $hostTarget
            if ($LASTEXITCODE -ne 0) { throw 'Could not build the Rust cache manager.' }
            $helperName = if ($env:OS -eq 'Windows_NT') { 'bevy-cargo-cache.exe' } else { 'bevy-cargo-cache' }
            $helper = Join-Path $helperTarget "$hostTarget/debug/$helperName"
        }
    } finally {
        $buildLock.Dispose()
    }
    if ($helper) {
        & $helper $cargoPath $targetPath @cargoArgs
        $result = $LASTEXITCODE
    }
} finally {
    Pop-Location
}
exit $result
