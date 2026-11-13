@echo off
setlocal
cd /d "%~dp0"

rem Boot-time fix: explorer.exe (desktop shell / WorkerW) and the WebView2 runtime
rem may not be ready when the autostart script calls this file. Creating the window
rem too early leaves the webview on an error page that never retries by itself.
rem Wait for the shell, then give WebView2 a short buffer before launching.
rem The app also has a frontend-ready watchdog that reloads the webview if the page
rem still failed to load, so this delay only has to be approximately right.

set /a TRIES=0
:wait_shell
tasklist /fi "imagename eq explorer.exe" /nh 2>nul | find /i "explorer.exe" >nul
if not errorlevel 1 goto shell_ready
set /a TRIES+=1
if %TRIES% GEQ 60 goto launch
ping -n 2 127.0.0.1 >nul
goto wait_shell

:shell_ready
ping -n 4 127.0.0.1 >nul

:launch
call npm run tauri dev
