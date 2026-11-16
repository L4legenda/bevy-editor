param([string]$RepositoryRoot = (Resolve-Path "$PSScriptRoot/../../..").Path)
$ErrorActionPreference = 'Stop'
$nativeRoot = Join-Path $RepositoryRoot 'native/uv'
$sdkRoot = Join-Path $RepositoryRoot 'target/partuv-native/deps'
$stageRoot = Join-Path $RepositoryRoot 'target/partuv-native/header-stage'
New-Item -ItemType Directory -Force $stageRoot | Out-Null
function Copy-Headers([string]$Source, [string]$Destination) {
    New-Item -ItemType Directory -Force $Destination | Out-Null
    Get-ChildItem -LiteralPath $Source -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $Destination -Recurse -Force
    }
}
Copy-Headers "$sdkRoot/eigen/eigen-3.4.0/Eigen" "$stageRoot/Eigen"
Copy-Headers "$sdkRoot/eigen/eigen-3.4.0/unsupported" "$stageRoot/unsupported"
Copy-Headers "$sdkRoot/libigl/libigl-2.5.0/include/igl" "$stageRoot/igl"
Copy-Headers "$sdkRoot/boost/boost_1_87_0/boost" "$stageRoot/boost"
New-Item -ItemType Directory -Force "$stageRoot/CGAL" | Out-Null
Get-ChildItem -LiteralPath "$sdkRoot/cgal/cgal-6.0.1" -Directory | Sort-Object Name | ForEach-Object {
    $include = Join-Path $_.FullName 'include/CGAL'
    if (Test-Path -LiteralPath $include) { Copy-Headers $include "$stageRoot/CGAL" }
}
Copy-Headers "$RepositoryRoot/target/partuv-native/windows-source/extern/OpenABF/include/OpenABF" "$stageRoot/OpenABF"
Copy-Headers "$RepositoryRoot/target/partuv-native/windows-source/extern/easy_profiler/include/easy" "$stageRoot/easy"
New-Item -ItemType Directory -Force "$stageRoot/licenses/eigen", "$stageRoot/licenses/boost", "$stageRoot/licenses/cgal", "$stageRoot/licenses/libigl" | Out-Null
Get-ChildItem "$sdkRoot/eigen/eigen-3.4.0" -Filter 'COPYING*' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination "$stageRoot/licenses/eigen" }
Copy-Item -LiteralPath "$sdkRoot/boost/boost_1_87_0/LICENSE_1_0.txt" -Destination "$stageRoot/licenses/boost"
Get-ChildItem "$sdkRoot/cgal/cgal-6.0.1" -Filter 'LICENSE*' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination "$stageRoot/licenses/cgal" -Recurse -Force }
Get-ChildItem "$sdkRoot/libigl/libigl-2.5.0" -Filter 'LICENSE*' | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination "$stageRoot/licenses/libigl" }
& "$env:SystemRoot/System32/tar.exe" -a -c -f "$nativeRoot/vendor/headers.zip" -C $stageRoot .
if ($LASTEXITCODE -ne 0) { throw 'Header archive failed' }
Get-FileHash -LiteralPath "$nativeRoot/vendor/headers.zip" -Algorithm SHA256
Get-Item "$nativeRoot/vendor/headers.zip" | Select-Object Length
