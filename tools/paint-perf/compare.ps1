param(
    [string]$Before = 'target/paint-perf/bevy-before.exe',
    [string]$After = 'target/paint-perf/bevy-after.exe',
    [int]$Rounds = 3
)
$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Set-Location -LiteralPath $taskRoot
$perfDir = Join-Path $taskRoot 'target/paint-perf'
$mp = Get-ChildItem -LiteralPath (Join-Path $perfDir 'mp-build/debug/deps') -Filter 'material_painter-*.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$env:PAINT_PERF_DIR = $perfDir
$env:PAINT_PERF_SELECTED = '7'
$cases = @(
    @{ Name = 'before'; Exe = (Resolve-Path -LiteralPath $Before).Path; Test = 'gpu_compare_complex_paint' },
    @{ Name = 'material-painter'; Exe = $mp.FullName; Test = 'compare_complex_paint' },
    @{ Name = 'after'; Exe = (Resolve-Path -LiteralPath $After).Path; Test = 'gpu_compare_complex_paint' }
)
$rows = @()
for ($round = 0; $round -lt $Rounds; $round++) {
    # Rotate the order so warm adapter/clock effects are shared.
    for ($i = 0; $i -lt $cases.Count; $i++) {
        $case = $cases[($i + $round) % $cases.Count]
        $env:PAINT_PERF_LABEL = "$($case.Name)-round-$round"
        $log = Join-Path $perfDir "$($case.Name)-round-$round.log"
        & $case.Exe $case.Test --ignored --nocapture --test-threads=1 *> $log
        if ($LASTEXITCODE -ne 0) { throw "Benchmark failed: $log" }
        foreach ($line in Get-Content -LiteralPath $log) {
            if ($line -match 'PAINT_COMPARE (\{.+\})') {
                $row = $Matches[1] | ConvertFrom-Json
                $row | Add-Member -NotePropertyName round -NotePropertyValue $round
                $row.app = $case.Name
                $rows += $row
                Write-Output "$($case.Name) round $round, $($row.texture): $([math]::Round($row.mean_ms,3)) ms, p95 $([math]::Round($row.p95_ms,3)) ms"
            }
        }
    }
}
$rows | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $perfDir 'comparison.json') -Encoding utf8
