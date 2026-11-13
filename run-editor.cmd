@echo off
wscript.exe //nologo "%~dp0run-editor.vbs" %*
exit /b %errorlevel%
