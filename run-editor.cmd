@echo off
cscript.exe //nologo "%~dp0run-editor.vbs" %*
if errorlevel 1 (
    echo.
    echo Bevy Editor could not start. Check the build output above.
    pause
    exit /b 1
)
exit /b 0
