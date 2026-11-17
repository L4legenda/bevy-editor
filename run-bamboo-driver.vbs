Option Explicit
Dim fso, shell, root, executable, script, result, needsBuild, sourceFile
Set fso = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")
root = fso.GetParentFolderName(WScript.ScriptFullName)
executable = fso.BuildPath(root, "target\bamboo-driver\BambooDriver.exe")
script = fso.BuildPath(root, "tools\bamboo-driver\build.ps1")
needsBuild = Not fso.FileExists(executable)
If Not needsBuild Then
    For Each sourceFile In Array("BambooDriver.cs", "make-icon.ps1", "build.ps1", "bamboo.svg")
        If fso.GetFile(executable).DateLastModified < fso.GetFile(fso.BuildPath(root, "tools\bamboo-driver\" & sourceFile)).DateLastModified Then needsBuild = True
    Next
End If
If needsBuild Then
    result = shell.Run("powershell.exe -NoProfile -ExecutionPolicy Bypass -File " & Chr(34) & script & Chr(34), 0, True)
    If result <> 0 Then
        MsgBox "Bamboo driver build failed.", 16, "Bamboo CTE-450"
        WScript.Quit result
    End If
End If
shell.Run Chr(34) & executable & Chr(34), 1, False
