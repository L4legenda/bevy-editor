$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$logs = Join-Path $env:LOCALAPPDATA 'BevyEditor\logs'
$log = Join-Path $logs 'launcher.log'

try {
    New-Item -ItemType Directory -Path $logs -Force | Out-Null
    $powershell = Join-Path $PSHOME 'powershell.exe'
    try {
        # Windows PowerShell exposes native stderr as error records, including Cargo progress.
        $ErrorActionPreference = 'Continue'
        & $powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'build-editor.ps1') 2>&1 |
            ForEach-Object { $_.ToString() } |
            Tee-Object -FilePath $log -ErrorAction Stop
    } finally {
        $ErrorActionPreference = 'Stop'
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Build failed (exit code $LASTEXITCODE)."
    }

    $editor = Join-Path $repo 'target\debug\bevy-editor.exe'
    if (-not (Test-Path -LiteralPath $editor -PathType Leaf)) {
        throw "Build did not produce $editor."
    }

    $start = @{
        FilePath = $editor
        WorkingDirectory = (Get-Location).ProviderPath
    }
    if ($args.Count -gt 0) {
        # Windows argv needs escaped quotes and doubled backslashes before quotes.
        $start.ArgumentList = ($args | ForEach-Object {
            '"' + ([string]$_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"'
        }) -join ' '
    }
    Write-Output 'Starting Bevy Editor...'
    Start-Process @start
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    [Console]::Error.WriteLine("Launcher log: $log")
    exit 1
}
