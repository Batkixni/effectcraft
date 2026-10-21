@echo off
if not exist "%TEMP%\ec-aexlo-pixel-28e9f9a.stamp" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0patch-aexlo-pixel.ps1"
  if errorlevel 1 exit /b 1
  echo patched>"%TEMP%\ec-aexlo-pixel-28e9f9a.stamp"
)
%*
