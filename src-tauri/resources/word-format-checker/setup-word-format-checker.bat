@echo off
setlocal EnableExtensions

set "REQUIREMENTS=%~dp0requirements.txt"
set "PYTHON_VERSION=3.13.14"
set "PYTHON_DIR=%LocalAppData%\Programs\Python\Python313"
set "PYTHON_EXE=%PYTHON_DIR%\python.exe"
set "PYTHON_INSTALLER=%TEMP%\python-%PYTHON_VERSION%-amd64.exe"
set "PYTHON_URL=https://www.python.org/ftp/python/%PYTHON_VERSION%/python-%PYTHON_VERSION%-amd64.exe"
set "EXIT_CODE=0"

echo.
echo Preparing technical bid format checker dependencies...
echo.

if not exist "%REQUIREMENTS%" goto requirements_missing

call :find_python
if defined PYTHON_CMD goto install_dependencies

echo Python 3.8 or later was not found.
echo Downloading and installing Python %PYTHON_VERSION% for the current user...
powershell -NoProfile -ExecutionPolicy Bypass -Command "Invoke-WebRequest -UseBasicParsing -Uri '%PYTHON_URL%' -OutFile '%PYTHON_INSTALLER%'"
if errorlevel 1 goto download_failed

"%PYTHON_INSTALLER%" /quiet InstallAllUsers=0 PrependPath=1 Include_pip=1
if errorlevel 1 goto python_install_failed

call :find_python
if not defined PYTHON_CMD goto python_not_found

:install_dependencies
echo Installing dependencies. Please wait...
%PYTHON_CMD% -m ensurepip --upgrade
if errorlevel 1 goto dependency_failed
%PYTHON_CMD% -m pip install --user -r "%REQUIREMENTS%"
if errorlevel 1 goto dependency_failed
%PYTHON_CMD% -c "import docx"
if errorlevel 1 goto verification_failed

echo.
echo Setup completed successfully.
goto end

:find_python
set "PYTHON_CMD="
python -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 1)" >nul 2>nul && set "PYTHON_CMD=python"
if defined PYTHON_CMD exit /b
py -3 -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 1)" >nul 2>nul && set "PYTHON_CMD=py -3"
if defined PYTHON_CMD exit /b
if exist "%PYTHON_EXE%" set "PYTHON_CMD="%PYTHON_EXE%""
exit /b

:requirements_missing
set "EXIT_CODE=1"
echo requirements.txt was not found. Please run this script from the installed app resources folder.
goto end

:download_failed
set "EXIT_CODE=1"
echo Python installer download failed. Please check the network and retry.
goto end

:python_install_failed
set "EXIT_CODE=1"
echo Python installation failed. Please run this script again as the current Windows user.
goto end

:python_not_found
set "EXIT_CODE=1"
echo Python was installed but was not detected. Close this window and run this script again.
goto end

:dependency_failed
set "EXIT_CODE=1"
echo Dependency installation failed. Please check the network and retry.
goto end

:verification_failed
set "EXIT_CODE=1"
echo Dependency verification failed. Please run this script again.

:end
if exist "%PYTHON_INSTALLER%" del "%PYTHON_INSTALLER%" >nul 2>nul
echo.
pause
exit /b %EXIT_CODE%
