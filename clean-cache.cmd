@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\cargo.ps1" clean
if errorlevel 1 (
    echo.
    echo Cache cleanup failed. Check the message above.
    pause
    exit /b 1
)
echo.
echo Build cache cleaned. The next editor launch will rebuild the application.
pause
exit /b 0
