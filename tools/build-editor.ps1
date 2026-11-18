# Prepare everything needed by the source-checkout Windows launcher.
$ErrorActionPreference = 'Stop'
$scriptRoot = $PSScriptRoot
if ($scriptRoot.StartsWith('\\?\UNC\')) { $scriptRoot = '\\' + $scriptRoot.Substring(8) }
elseif ($scriptRoot.StartsWith('\\?\')) { $scriptRoot = $scriptRoot.Substring(4) }
$repo = [IO.Path]::GetFullPath([IO.Path]::Combine($scriptRoot, '..'))
$shellName = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh.exe' } else { 'powershell.exe' }
$shellPath = Join-Path $PSHOME $shellName
$prepare = $env:BEVY_EDITOR_PREPARE_3D
if ($prepare -and $prepare -notin @('0', '1')) { throw 'BEVY_EDITOR_PREPARE_3D must be 0 or 1.' }
$nvidia = ($prepare -eq '1') -or ((-not $prepare) -and (Test-Path -LiteralPath (Join-Path $env:SystemRoot 'System32/nvcuda.dll')))
if ($prepare -eq '0') { $nvidia = $false }

Write-Output 'Bevy Editor: checking the editor build...'
& $shellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $scriptRoot 'cargo.ps1') build --locked --manifest-path (Join-Path $repo 'Cargo.toml') --bin bevy-editor
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($nvidia) {
    Write-Output 'Bevy Editor: preparing the Rust 3D generator and CUDA libraries...'
    Write-Output 'The first preparation may download the CUDA toolkit and compile GPU kernels. Model weights are loaded only from the editor.'
    $lockPath = Join-Path $repo '.bevy-editor-build.lock'
    if ((Test-Path -LiteralPath $lockPath) -and ((Get-Item -LiteralPath $lockPath -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Refusing to use a linked build lock.'
    }
    try {
        $buildLock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    } catch { throw 'Another editor build is active. Wait for it to finish before starting another.' }
    try {
        $destination = Join-Path $repo 'target/debug'
        if ((Test-Path -LiteralPath $destination) -and ((Get-Item -LiteralPath $destination -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Refusing to prepare the generator in a linked directory.'
        }
        & $shellPath -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $scriptRoot 'build-hunyuan.ps1') -Destination $destination
        if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the 3D generator. See the preceding CUDA/Cargo error.' }
        foreach ($name in @('bevy-hunyuan.exe', 'cudart64_12.dll', 'cublas64_12.dll', 'cublasLt64_12.dll', 'curand64_10.dll', 'nvrtc64_120_0.dll', 'nvrtc-builtins64_128.dll')) {
            if (-not (Test-Path -LiteralPath (Join-Path $destination $name) -PathType Leaf)) { throw "3D generator preparation did not produce $name" }
        }
    } finally { $buildLock.Dispose() }
    Write-Output 'Bevy Editor: 3D generator is ready.'
} else {
    Write-Output 'Bevy Editor: CUDA 3D generation is disabled or no NVIDIA driver was found.'
}
exit 0
