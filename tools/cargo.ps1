# Track Cargo's current artifacts and prune only indexed, superseded files.
# Examples: .\tools\cargo.ps1 build --locked --bin bevy-editor
#           .\tools\cargo.ps1 test --locked
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
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
$python = Get-Command python -ErrorAction Stop

# Serialise launcher/build-script calls, including maintenance and compilation.
$lockPath = Join-Path $projectRoot '.bevy-editor-build.lock'
try {
    $buildLock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
} catch {
    throw 'Another editor build is active. Wait for it to finish before starting another.'
}
$previousTarget = $env:CARGO_TARGET_DIR
Push-Location -LiteralPath $projectRoot
try {
    $env:CARGO_TARGET_DIR = $targetPath
    if (Test-Path -LiteralPath $targetPath) {
        $targetItem = Get-Item -LiteralPath $targetPath -Force
        if ($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'Refusing to maintain a target directory that is a link or junction.'
        }
    }
    $limitText = $limitGB.ToString([Globalization.CultureInfo]::InvariantCulture)
    & $python.Source (Join-Path $PSScriptRoot 'cargo_cache.py') $cargoPath $targetPath $limitText @cargoArgs
    $result = $LASTEXITCODE
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
    $buildLock.Dispose()
}
exit $result
