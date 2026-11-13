Option Explicit

Dim files, shell, root, editor, arguments, index
Set files = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")
root = files.GetParentFolderName(WScript.ScriptFullName)
editor = files.BuildPath(root, "target\debug\bevy-editor.exe")

If Not files.FileExists(editor) Then
    Dim logs, logFile, cargo, command, result
    logs = shell.ExpandEnvironmentStrings("%LOCALAPPDATA%")
    logs = files.BuildPath(logs, "BevyEditor")
    If Not files.FolderExists(logs) Then files.CreateFolder logs
    logs = files.BuildPath(logs, "logs")
    If Not files.FolderExists(logs) Then files.CreateFolder logs
    logFile = files.BuildPath(logs, "launcher.log")
    cargo = shell.ExpandEnvironmentStrings("%USERPROFILE%\.cargo\bin\cargo.exe")
    If Not files.FileExists(cargo) Then cargo = "cargo"
    command = QuoteArgument(cargo) & " build --locked --manifest-path " _
        & QuoteArgument(files.BuildPath(root, "Cargo.toml")) & " --bin bevy-editor >" _
        & QuoteArgument(logFile) & " 2>&1"
    result = shell.Run(QuoteArgument(shell.ExpandEnvironmentStrings("%COMSPEC%")) _
        & " /d /s /c " & Chr(34) & command & Chr(34), 0, True)
    If result <> 0 Or Not files.FileExists(editor) Then
        MsgBox "Could not build Bevy Editor. Rust 1.97.1+ and C++ Build Tools are required." _
            & vbCrLf & vbCrLf & "Build log: " & logFile, vbOKOnly + vbCritical, "Bevy Editor"
        WScript.Quit 1
    End If
End If

arguments = ""
For index = 0 To WScript.Arguments.Count - 1
    arguments = arguments & " " & QuoteArgument(WScript.Arguments(index))
Next
shell.CurrentDirectory = root
shell.Run QuoteArgument(editor) & arguments, 1, False

' Windows argv quoting: preserve spaces, quotes and trailing backslashes.
Function QuoteArgument(value)
    Dim quoted, slashes, position, character
    quoted = Chr(34)
    slashes = 0
    For position = 1 To Len(value)
        character = Mid(value, position, 1)
        If character = "\" Then
            slashes = slashes + 1
        ElseIf character = Chr(34) Then
            quoted = quoted & String(slashes * 2 + 1, "\") & Chr(34)
            slashes = 0
        Else
            quoted = quoted & String(slashes, "\") & character
            slashes = 0
        End If
    Next
    QuoteArgument = quoted & String(slashes * 2, "\") & Chr(34)
End Function
