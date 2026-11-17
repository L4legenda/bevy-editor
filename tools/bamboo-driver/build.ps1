$ErrorActionPreference = 'Stop'
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $compiler)) { throw '.NET Framework 4.x compiler is unavailable.' }
$source = Join-Path $PSScriptRoot 'BambooDriver.cs'
$output = Join-Path $PSScriptRoot '..\..\target\bamboo-driver'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$exe = Join-Path $output 'BambooDriver.exe'
$icon = Join-Path $output 'BambooDriver.ico'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'make-icon.ps1') -Output $icon
if ($LASTEXITCODE -ne 0) { throw 'Bamboo icon generation failed.' }
& $compiler /nologo /target:winexe /platform:x64 /optimize+ /codepage:65001 /reference:System.Windows.Forms.dll /reference:System.Drawing.dll "/win32icon:$icon" "/resource:$icon,BambooDriver.ico" "/out:$exe" $source
if ($LASTEXITCODE -ne 0) { throw 'Bamboo driver build failed.' }
Write-Output $exe
