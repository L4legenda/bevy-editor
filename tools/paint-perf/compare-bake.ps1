$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$directory = Join-Path $root 'target/paint-perf'
$previousDirectory = $env:PAINT_PERF_DIR
$previousBackend = $env:WGPU_BACKEND
$previousQueueMode = $env:PAINT_BAKE_SERIAL
Push-Location -LiteralPath $root
try {
    $env:PAINT_PERF_DIR = $directory
    $env:WGPU_BACKEND = 'vulkan'
    $env:PAINT_BAKE_SERIAL = '0'
    $cases = @(
        @{
            Log = 'bake-bevy.log'
            Arguments = @('--manifest-path', 'Cargo.toml', '--lib', '--config', 'profile.test.opt-level=1', '--config', 'profile.test.debug=0', '--config', 'profile.test.package.bevy-editor.opt-level=1', 'gpu_compare_mesh_bake')
        },
        @{
            Log = 'bake-material-painter.log'
            Arguments = @('--manifest-path', 'target/paint-perf/material-painter/Cargo.toml', '--target-dir', 'target/paint-perf/mp-build', '--bin', 'material_painter', 'compare_mesh_bake')
        }
    )
    $rows = foreach ($case in $cases) {
        $log = Join-Path $directory $case.Log
        $arguments = $case.Arguments
        & cargo test --locked --offline @arguments -- --ignored --nocapture --test-threads=1 *> $log
        if ($LASTEXITCODE -ne 0) { throw "Bake benchmark failed. See $log" }
        foreach ($line in Get-Content -LiteralPath $log) {
            if ($line -match 'BAKE_COMPARE (\{.+\})') {
                $Matches[1] | ConvertFrom-Json
            }
        }
    }
    $adapters = @($rows | Group-Object adapter, backend)
    if ($adapters.Count -ne 1) { throw 'The benchmarks must run on the same adapter and graphics API.' }
    $groups = @($rows | Group-Object app, size, high)
    if ($groups.Count -ne 6) { throw 'Missing benchmark cases.' }
    $summary = foreach ($group in $groups) {
        $samples = @($group.Group | Sort-Object total_ms)
        if ($samples.Count -ne 3) { throw "Expected three measured runs for $($group.Name)." }
        [PSCustomObject]@{
            App = $samples[1].app
            Size = $samples[1].size
            High = $samples[1].high
            MinimumMs = $samples[0].total_ms
            MedianMs = $samples[1].total_ms
            MaximumMs = $samples[2].total_ms
        }
    }
    $report = @{ Adapter = $adapters[0].Name; Quality = 'Standard'; Samples = $rows; Summary = $summary }
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $directory 'bake-comparison.json') -Encoding utf8
    $summary | Format-Table -AutoSize
} finally {
    Pop-Location
    $env:PAINT_PERF_DIR = $previousDirectory
    $env:WGPU_BACKEND = $previousBackend
    $env:PAINT_BAKE_SERIAL = $previousQueueMode
}
