param([switch]$Release, [string]$Destination)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
function Copy-Component([string]$SourcePath, [string]$DestinationDirectory) {
    $source = Get-Item -LiteralPath $SourcePath -ErrorAction Stop
    $target = Join-Path $DestinationDirectory $source.Name
    $existing = Get-Item -LiteralPath $target -ErrorAction SilentlyContinue
    if ($existing -and $existing.Length -eq $source.Length -and $existing.LastWriteTimeUtc -eq $source.LastWriteTimeUtc) { return }
    Copy-Item -LiteralPath $source.FullName -Destination $target -Force
}
# This is a developer build helper, never used to run inference. The shipped worker
# and image/model processing are Rust; CUDA redistributables are NVIDIA libraries.
$cudaRoot = $env:CUDA_PATH
if (-not $cudaRoot -or -not (Test-Path (Join-Path $cudaRoot 'bin/nvcc.exe')) -or -not (Test-Path (Join-Path $cudaRoot 'bin/nvrtc-builtins64_128.dll'))) {
    $cache = Join-Path $env:LOCALAPPDATA 'BevyEditor/system/runtime/cuda-12.8.1'
    $cudaRoot = Join-Path $cache 'toolkit'
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $cache '.gitignore'), "*`n!.gitignore`n")
    $packages = @(
        @('cuda_nvcc', 'cuda_nvcc/windows-x86_64/cuda_nvcc-windows-x86_64-12.8.93-archive.zip', '9fdc70b4271ed9aad4d64cd7076a7d96ec36512d074b9995fe638de669197391'),
        @('cuda_cudart', 'cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.8.90-archive.zip', '4a39058fd8519444a81cfc7ae055d136f48d1a31ffa41ae255b35b2edd61e13b'),
        @('cuda_cccl', 'cuda_cccl/windows-x86_64/cuda_cccl-windows-x86_64-12.8.90-archive.zip', 'bd8548fa1ae82f92910bebc3079e14bd58c5a92aa64596d46bd610a478cb39d7'),
        @('cuda_nvrtc', 'cuda_nvrtc/windows-x86_64/cuda_nvrtc-windows-x86_64-12.8.93-archive.zip', 'a63302a077f0248a743a1a7caa7dbd80d0fac56c6cfa9c41fa05fac9b7e5eda5'),
        @('libcublas', 'libcublas/windows-x86_64/libcublas-windows-x86_64-12.8.4.1-archive.zip', '57a470112cec7e112c95253dde8b3c7184d795dbd92b0bde77a4cb7f8c94c8aa'),
        @('libcurand', 'libcurand/windows-x86_64/libcurand-windows-x86_64-10.3.9.90-archive.zip', '46eba36c20748b21a5927b569b943337dd18485132119dfb39a5bcda4edb12e2')
    )
    foreach ($package in $packages) {
        $folder = Join-Path $cache $package[0]
        if (-not (Test-Path (Join-Path $folder '.ready'))) {
            $zip = Join-Path $cache ($package[0] + '.zip')
            if (-not (Test-Path $zip) -or (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $package[2]) {
                Write-Host ('Downloading NVIDIA ' + $package[0])
                Invoke-WebRequest ('https://developer.download.nvidia.com/compute/cuda/redist/' + $package[1]) -OutFile $zip
            }
            if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $package[2]) { throw 'CUDA SHA-256 mismatch' }
            Expand-Archive -LiteralPath $zip -DestinationPath $folder -Force
            $archive = Get-ChildItem -LiteralPath $folder -Directory | Select-Object -First 1
            New-Item -ItemType Directory -Path $cudaRoot -Force | Out-Null
            Get-ChildItem -LiteralPath $archive.FullName | Copy-Item -Destination $cudaRoot -Recurse -Force
            [IO.File]::WriteAllText((Join-Path $folder '.ready'), $package[2])
        }
    }
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsInstall = & $vswhere -latest -products '*' -property installationPath
if (-not $vsInstall) { throw 'Install Visual Studio C++ build tools to compile Candle CUDA kernels' }
$dev = Join-Path $vsInstall 'Common7/Tools/VsDevCmd.bat'
$environmentCommand = '"' + $dev + '" -arch=x64 -host_arch=x64 >nul && set'
cmd /c $environmentCommand | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') }
}
$env:CUDA_PATH = $cudaRoot
$env:PATH = (Join-Path $cudaRoot 'bin') + ';' + $env:PATH
$env:NVCC_PREPEND_FLAGS = '-allow-unsupported-compiler'
# PTX can JIT on all supported Ampere-and-newer GPUs, including 3060 Ti.
if (-not $env:CUDA_COMPUTE_CAP) { $env:CUDA_COMPUTE_CAP = '80' }
$env:RAYON_NUM_THREADS = '2'
$arguments = @('build', '--locked', '--manifest-path', (Join-Path $repo 'crates/hunyuan-native/Cargo.toml'), '--target-dir', (Join-Path $repo 'crates/hunyuan-native/target'))
$nativeProfile = 'debug'
if ($Release) { $arguments += '--release'; $nativeProfile = 'release' }
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw 'Native Hunyuan build failed' }
if ($Destination) {
    $destinationPath = [IO.Path]::GetFullPath($Destination)
    New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
    Copy-Component (Join-Path $repo "crates/hunyuan-native/target/$nativeProfile/bevy-hunyuan.exe") $destinationPath
    foreach ($name in @('cudart64_12.dll', 'cublas64_12.dll', 'cublasLt64_12.dll', 'curand64_10.dll', 'nvrtc64_120_0.dll', 'nvrtc-builtins64_128.dll')) {
        Copy-Component (Join-Path $cudaRoot "bin/$name") $destinationPath
    }
    Copy-Component (Join-Path $repo 'crates/hunyuan-native/LICENSE-TENCENT') $destinationPath
    Copy-Component (Join-Path $repo 'crates/hunyuan-native/NOTICE') $destinationPath
    $licenses = Join-Path $destinationPath 'cuda-licenses'
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
    Get-ChildItem -LiteralPath $cudaRoot -File | Where-Object { $_.Name -match 'LICENSE|EULA' } | Copy-Item -Destination $licenses -Force
}
