Option Explicit

Dim files, shell, root, editor, arguments, index, console, launchBuilt
Set files = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")
root = files.GetParentFolderName(WScript.ScriptFullName)
editor = files.BuildPath(root, "target\debug\bevy-editor.exe")
shell.CurrentDirectory = root
console = LCase(files.GetFileName(WScript.FullName)) = "cscript.exe"
launchBuilt = False
If Not console And WScript.Arguments.Count > 0 Then
    If WScript.Arguments(0) = "--launch-built" Then
        launchBuilt = True
    End If
End If

' Always let Cargo check freshness after a pull, even if an older exe exists.
Dim logs, logFile, command, result, build, output, line, handoff, saved, count, length, value
logs = shell.ExpandEnvironmentStrings("%LOCALAPPDATA%")
logs = files.BuildPath(logs, "BevyEditor")
If Not files.FolderExists(logs) Then files.CreateFolder logs
logs = files.BuildPath(logs, "logs")
If Not files.FolderExists(logs) Then files.CreateFolder logs
logFile = files.BuildPath(logs, "launcher.log")
If Not launchBuilt Then
    command = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File " _
        & QuoteArgument(files.BuildPath(root, "tools\build-editor.ps1"))
    If console Then
        WScript.Echo "Bevy Editor: checking the editor and native components..."
        WScript.Echo "First-time CUDA preparation and compilation can take several minutes. Progress follows."
        WScript.Echo "Please keep this window open until: Starting Bevy Editor..."
        Set output = files.CreateTextFile(logFile, True, True)
        Set build = shell.Exec(QuoteArgument(shell.ExpandEnvironmentStrings("%COMSPEC%")) _
            & " /d /s /c " & Chr(34) & command & " 2>&1" & Chr(34))
        build.StdIn.Close
        Do Until build.StdOut.AtEndOfStream
            line = build.StdOut.ReadLine
            WScript.Echo line
            output.WriteLine line
        Loop
        Do While build.Status = 0
            WScript.Sleep 20
        Loop
        output.Close
        result = build.ExitCode
    Else
        command = command & " >" & QuoteArgument(logFile) & " 2>&1"
        result = shell.Run(QuoteArgument(shell.ExpandEnvironmentStrings("%COMSPEC%")) _
            & " /d /s /c " & Chr(34) & command & Chr(34), 0, True)
    End If
    If result <> 0 Or Not files.FileExists(editor) Then
        If console Then
            WScript.Echo "Build failed. Log: " & logFile
        Else
            MsgBox "Could not build Bevy Editor. See the build log for details." _
                & vbCrLf & vbCrLf & "Build log: " & logFile, vbOKOnly + vbCritical, "Bevy Editor"
        End If
        WScript.Quit 1
    End If
End If

arguments = ""
If launchBuilt Then
    handoff = WScript.Arguments(1)
    ' Only consume our own temporary argument file. Never delete a project argument.
    If LCase(files.GetParentFolderName(files.GetAbsolutePathName(handoff))) <> LCase(logs) _
        Or LCase(Left(files.GetFileName(handoff), 7)) <> "launch-" Then WScript.Quit 1
    Set saved = files.OpenTextFile(handoff, 1, False, -1)
    count = CInt(saved.ReadLine)
    For index = 0 To count - 1
        length = CInt(saved.ReadLine)
        value = ""
        If length > 0 Then value = saved.Read(length)
        arguments = arguments & " " & QuoteArgument(value)
    Next
    saved.Close
    files.DeleteFile handoff
Else
    For index = 0 To WScript.Arguments.Count - 1
        arguments = arguments & " " & QuoteArgument(WScript.Arguments(index))
    Next
End If
If console Then
    WScript.Echo "Starting Bevy Editor..."
    ' A GUI host launches the editor so AttachConsole cannot keep this terminal open.
    ' WSH parses quotes differently from Windows argv; transfer arguments as Unicode
    ' text instead of quoting user paths through a second script host.
    handoff = files.BuildPath(logs, "launch-" & files.GetTempName)
    Set saved = files.CreateTextFile(handoff, True, True)
    saved.WriteLine WScript.Arguments.Count
    For index = 0 To WScript.Arguments.Count - 1
        value = WScript.Arguments(index)
        saved.WriteLine Len(value)
        saved.Write value
    Next
    saved.Close
    shell.Run QuoteArgument(files.BuildPath(files.GetParentFolderName(WScript.FullName), "wscript.exe")) _
        & " //nologo " & QuoteArgument(WScript.ScriptFullName) & " --launch-built " & QuoteArgument(handoff), 0, False
Else
    shell.Run QuoteArgument(editor) & arguments, 1, False
End If

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
