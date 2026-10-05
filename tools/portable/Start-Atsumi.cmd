@echo off
setlocal
if exist "%~dp0WebView2\msedgewebview2.exe" set "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER=%~dp0WebView2"
start "" /D "%~dp0" "%~dp0atsumi.exe"
endlocal
