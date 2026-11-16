# Build/run the independent Rust cache manager. Python is not required.
# Examples: .\tools\cargo.ps1 build --locked --bin bevy-editor
#           .\tools\cargo.ps1 test --locked
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1's filesystem provider cannot Join-Path a verbatim drive
# returned by Rust canonicalize; use its equivalent ordinary Windows spelling.
$scriptRoot = $PSScriptRoot
if ($scriptRoot.StartsWith('\\?\UNC\')) { $scriptRoot = '\\' + $scriptRoot.Substring(8) }
elseif ($scriptRoot.StartsWith('\\?\')) { $scriptRoot = $scriptRoot.Substring(4) }
$projectRoot = [IO.Path]::GetFullPath([IO.Path]::Combine($scriptRoot, '..'))
$targetPath = Join-Path $projectRoot 'target'
$cargoArgs = @($args)
if ($cargoArgs.Count -eq 0) { $cargoArgs = @('build', '--locked', '--bin', 'bevy-editor') }
if ($cargoArgs[0] -notin @('build', 'check', 'test', 'run', 'clippy')) {
    throw 'Supported commands: build, check, test, run, clippy.'
}

$limitGB = 6.0
if ($env:BEVY_EDITOR_TARGET_LIMIT_GB) {
    $limitGB = [double]::Parse($env:BEVY_EDITOR_TARGET_LIMIT_GB, [Globalization.CultureInfo]::InvariantCulture)
    if ($limitGB -le 0 -or [double]::IsNaN($limitGB) -or [double]::IsInfinity($limitGB)) {
        throw 'BEVY_EDITOR_TARGET_LIMIT_GB must be a finite positive number.'
    }
}
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoPath = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargoPath)) { throw 'Cargo was not found. Install Rust first.' }
Push-Location -LiteralPath $projectRoot
try {
    if (Test-Path -LiteralPath $targetPath) {
        $targetItem = Get-Item -LiteralPath $targetPath -Force
        if ($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing to maintain a target directory that is a link or junction.'
        }
    }
    # Cover bootstrap with the same launcher lock. The Rust process acquires it
    # itself for the whole Cargo command and cache maintenance that follow.
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
    } finally {
        $buildLock.Dispose()
    }
    $limitText = $limitGB.ToString([Globalization.CultureInfo]::InvariantCulture)
    & $helper $cargoPath $targetPath $limitText @cargoArgs
    $result = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $result
