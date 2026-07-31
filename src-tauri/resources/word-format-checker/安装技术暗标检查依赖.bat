@echo off
setlocal EnableExtensions

call "%~dp0setup-word-format-checker.bat"
exit /b %ERRORLEVEL%
