param([string] $Output = 'target/paint-perf/path-trace/compare')

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$native = Join-Path $root 'target/paint-perf/material-painter'
$raytrace = Join-Path $native 'src/raytrace.rs'
if (-not (Test-Path -LiteralPath $raytrace)) {
    throw "The Material Painter comparison checkout is missing at $native."
}
$harness = (Join-Path $PSScriptRoot 'material_painter_pathtrace_compare.rs').Replace('\', '/')
if (-not (Select-String -LiteralPath $raytrace -SimpleMatch 'material_painter_pathtrace_compare.rs' -Quiet)) {
    Add-Content -LiteralPath $raytrace -Encoding utf8 -Value ('#[cfg(test)] include!(r#"' + $harness + '"#);')
}

$previousOutput = $env:PAINT_TRACE_DIR
$previousBackend = $env:WGPU_BACKEND
Push-Location -LiteralPath $root
try {
    New-Item -ItemType Directory -Path $Output -Force | Out-Null
    $env:PAINT_TRACE_DIR = (Resolve-Path -LiteralPath $Output).Path
    $env:WGPU_BACKEND = 'vulkan'
    & ./tools/cargo.ps1 test --locked --offline --lib --config profile.test.opt-level=1 --config profile.test.debug=0 --config profile.test.package.bevy-editor.opt-level=0 gpu_compare_paint_pathtrace '--' --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Bevy Editor path trace capture failed.' }
    & cargo test --manifest-path (Join-Path $native 'Cargo.toml') --target-dir target/paint-perf/mp-build --locked --offline --bin material_painter compare_paint_pathtrace -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Material Painter path trace capture failed.' }
    & python (Join-Path $PSScriptRoot 'compare-pathtrace.py') $env:PAINT_TRACE_DIR
    if ($LASTEXITCODE -ne 0) { throw 'Path trace comparison failed.' }
} finally {
    $env:PAINT_TRACE_DIR = $previousOutput
    $env:WGPU_BACKEND = $previousBackend
    Pop-Location
}
