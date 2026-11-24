$ErrorActionPreference = 'Stop'
$editorFile = $env:BEVY_UPDATE_EXE
$stagedFile = $env:BEVY_UPDATE_STAGED
try {
    $editorProcess = Get-Process -Id ([int]$env:BEVY_UPDATE_PID) -ErrorAction SilentlyContinue
    if ($editorProcess -and $editorProcess.Path -eq $editorFile -and -not $editorProcess.WaitForExit(30000)) {
        throw 'The editor did not close.'
    }
    # A sibling file on the same volume can replace the old file atomically.
    [IO.File]::Replace($stagedFile, $editorFile, [System.Management.Automation.Language.NullString]::Value)
    Start-Process -FilePath $editorFile
} catch {
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Bevy Editor')
} finally {
    if (Test-Path -LiteralPath $stagedFile) {
        Remove-Item -LiteralPath $stagedFile -ErrorAction SilentlyContinue
    }
}
