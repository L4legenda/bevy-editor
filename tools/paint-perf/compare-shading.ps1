param([string] $Output = 'target/paint-perf/shading-compare')

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$native = Join-Path $root 'target/paint-perf/material-painter'
$preview = Join-Path $native 'src/preview_test.rs'
if (-not (Test-Path -LiteralPath $preview)) {
    throw "The Material Painter comparison checkout is missing at $native."
}
$harness = (Join-Path $PSScriptRoot 'material_painter_shading_compare.rs').Replace('\', '/')
if (-not (Select-String -LiteralPath $preview -SimpleMatch 'material_painter_shading_compare.rs' -Quiet)) {
    Add-Content -LiteralPath $preview -Encoding utf8 -Value ('include!(r#"' + $harness + '"#);')
}

$previousOutput = $env:PAINT_SHADING_DIR
$previousBackend = $env:WGPU_BACKEND
Push-Location -LiteralPath $root
try {
    New-Item -ItemType Directory -Path $Output -Force | Out-Null
    $env:PAINT_SHADING_DIR = (Resolve-Path -LiteralPath $Output).Path
    $env:WGPU_BACKEND = 'vulkan'
    & ./tools/cargo.ps1 test --locked --offline --lib --config profile.test.opt-level=1 --config profile.test.debug=0 --config profile.test.package.bevy-editor.opt-level=0 gpu_compare_paint_shading_viewport '--' --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Bevy Editor viewport capture failed.' }
    & cargo test --manifest-path (Join-Path $native 'Cargo.toml') --target-dir target/paint-perf/mp-build --locked --offline --bin material_painter compare_paint_shading_viewport -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Material Painter viewport capture failed.' }
    & python (Join-Path $PSScriptRoot 'compare-shading.py') $env:PAINT_SHADING_DIR
    if ($LASTEXITCODE -ne 0) { throw 'Viewport comparison failed.' }
} finally {
    $env:PAINT_SHADING_DIR = $previousOutput
    $env:WGPU_BACKEND = $previousBackend
    Pop-Location
}
