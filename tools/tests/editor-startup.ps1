# Isolated launcher tests: no Bevy build, GPU computation, downloads or user assets.
$ErrorActionPreference = 'Stop'
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ('bevy-editor-startup-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $testRoot 'tools') -Force | Out-Null
$shellPath = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$savedPrepare = $env:BEVY_EDITOR_PREPARE_3D
$savedStage = $env:BEVY_STARTUP_TEST_STAGE
$savedTrace = $env:BEVY_STARTUP_TEST_TRACE
$savedMissing = $env:BEVY_STARTUP_TEST_MISSING
try {
    Copy-Item -LiteralPath (Join-Path $sourceRoot 'tools/build-editor.ps1') -Destination (Join-Path $testRoot 'tools/build-editor.ps1')
    [IO.File]::WriteAllText((Join-Path $testRoot 'Cargo.toml'), '[package]')
    $cache = @'
$root = Split-Path $PSScriptRoot -Parent
Add-Content -LiteralPath $env:BEVY_STARTUP_TEST_TRACE -Value 'editor'
if ($args -notcontains '--bin' -or $args -notcontains 'bevy-editor') { exit 20 }
if ($env:BEVY_STARTUP_TEST_STAGE -eq 'editor-failure') { exit 21 }
New-Item -ItemType Directory -Path (Join-Path $root 'target/debug') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $root 'target/debug/bevy-editor.exe'), 'editor fixture')
exit 0
'@
    $native = @'
param([string]$Destination)
Add-Content -LiteralPath $env:BEVY_STARTUP_TEST_TRACE -Value 'generator'
[Console]::Error.WriteLine('Native compiler progress fixture')
if ($env:BEVY_STARTUP_TEST_STAGE -eq 'native-failure') { exit 22 }
$root = Split-Path $PSScriptRoot -Parent
$locked = $false
try { $probe = [IO.File]::Open((Join-Path $root '.bevy-editor-build.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $probe.Dispose() } catch { $locked = $true }
if (-not $locked) { exit 23 }
foreach ($name in @('bevy-hunyuan.exe','cudart64_12.dll','cublas64_12.dll','cublasLt64_12.dll','curand64_10.dll','nvrtc64_120_0.dll','nvrtc-builtins64_128.dll')) {
    $file = Join-Path $Destination $name
    if ($name -eq $env:BEVY_STARTUP_TEST_MISSING) { if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }; continue }
    [IO.File]::WriteAllText($file, 'runtime fixture')
}
exit 0
'@
    [IO.File]::WriteAllText((Join-Path $testRoot 'tools/cargo.ps1'), $cache)
    [IO.File]::WriteAllText((Join-Path $testRoot 'tools/build-hunyuan.ps1'), $native)
    $env:BEVY_STARTUP_TEST_TRACE = Join-Path $testRoot 'trace.log'
    function Invoke-Case([string]$Label, [string]$Prepare, [string]$Stage, [bool]$Success, [string]$Trace) {
        $env:BEVY_EDITOR_PREPARE_3D = $Prepare
        $env:BEVY_STARTUP_TEST_STAGE = $Stage
        [IO.File]::WriteAllText($env:BEVY_STARTUP_TEST_TRACE, '')
        $stdout = Join-Path $testRoot "$Label.stdout.log"
        $stderr = Join-Path $testRoot "$Label.stderr.log"
        $psi = [Diagnostics.ProcessStartInfo]::new($shellPath)
        $psi.UseShellExecute = $false; $psi.CreateNoWindow = $true
        $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
        $psi.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + (Join-Path $testRoot 'tools/build-editor.ps1') + '"'
        $process = [Diagnostics.Process]::Start($psi)
        $outTask = $process.StandardOutput.ReadToEndAsync(); $errTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) { $process.Kill(); throw "$Label timed out" }
        $outText = $outTask.GetAwaiter().GetResult(); $errText = $errTask.GetAwaiter().GetResult()
        [IO.File]::WriteAllText($stdout, $outText); [IO.File]::WriteAllText($stderr, $errText)
        if (($process.ExitCode -eq 0) -ne $Success) { throw "$Label unexpected exit $($process.ExitCode): $outText $errText" }
        $actual = ((Get-Content -LiteralPath $env:BEVY_STARTUP_TEST_TRACE) -join ',')
        if ($actual -ne $Trace) { throw "$Label stage sequence: $actual, expected $Trace" }
        Write-Output "$Label passed"
    }
    Invoke-Case 'nvidia-startup' '1' '' $true 'editor,generator'
    Invoke-Case 'second-startup' '1' '' $true 'editor,generator'
    Invoke-Case 'cpu-startup' '0' '' $true 'editor'
    Invoke-Case 'editor-failure' '1' 'editor-failure' $false 'editor'
    Invoke-Case 'generator-failure' '1' 'native-failure' $false 'editor,generator'
    $env:BEVY_STARTUP_TEST_MISSING = 'cublasLt64_12.dll'
    Invoke-Case 'incomplete-runtime' '1' '' $false 'editor,generator'
    $env:BEVY_STARTUP_TEST_MISSING = ''
    Invoke-Case 'repaired-runtime' '1' '' $true 'editor,generator'
    $vbs = [IO.File]::ReadAllText((Join-Path $sourceRoot 'run-editor.vbs'))
    if (-not $vbs.Contains('tools\build-editor.ps1')) { throw 'VBS launcher does not prepare native components' }
    $cmd = [IO.File]::ReadAllText((Join-Path $sourceRoot 'run-editor.cmd'))
    if (-not $cmd.Contains('run-editor.vbs')) { throw 'CMD launcher bypasses the common VBS path' }
    Write-Output 'Both Windows launchers use the complete build pipeline.'
} finally {
    $env:BEVY_EDITOR_PREPARE_3D = $savedPrepare
    $env:BEVY_STARTUP_TEST_STAGE = $savedStage
    $env:BEVY_STARTUP_TEST_TRACE = $savedTrace
    $env:BEVY_STARTUP_TEST_MISSING = $savedMissing
    $resolved = [IO.Path]::GetFullPath($testRoot)
    if ((Split-Path $resolved -Parent).TrimEnd('\') -eq $tempBase.TrimEnd('\') -and (Split-Path $resolved -Leaf).StartsWith('bevy-editor-startup-')) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}
