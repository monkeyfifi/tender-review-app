# Python Dependency Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone Windows script that installs Python when needed and prepares the bundled technical blind-bid checker dependencies.

**Architecture:** Put the batch script in the already bundled `word-format-checker` resource directory so it can locate `requirements.txt` using `%~dp0`. The script discovers Python before downloading the official per-user Python 3.13.14 installer, then installs and verifies dependencies. A small Vitest static test protects the script contract and the guide points users to the script.

**Tech Stack:** Windows batch, Windows PowerShell `Invoke-WebRequest`, Python/pip, Vitest.

## Global Constraints

- Keep the implementation as a single standalone `.bat` file; do not change Rust review behavior.
- Use only built-in Windows commands and PowerShell; add no Node, Rust, or Python dependency.
- Download Python 3.13.14 only from `https://www.python.org/ftp/python/3.13.14/python-3.13.14-amd64.exe`.
- Install Python for the current user with `InstallAllUsers=0`, `PrependPath=1`, and `Include_pip=1`.
- Resolve the requirements file from `%~dp0requirements.txt`.
- Leave the terminal window open with a readable Chinese result message.

---

### Task 1: Add and protect the one-click dependency script

**Files:**
- Create: `src-tauri/resources/word-format-checker/安装技术暗标检查依赖.bat`
- Create: `tests/python-dependency-bootstrap.test.ts`

**Interfaces:**
- Consumes: `%~dp0requirements.txt`, Windows `python` or `py -3`, and PowerShell.
- Produces: A runnable batch file that exits `0` only after `import docx` succeeds.

- [ ] **Step 1: Write the failing static test**

```ts
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const script = readFileSync(
  new URL("../src-tauri/resources/word-format-checker/安装技术暗标检查依赖.bat", import.meta.url),
  "utf8",
);

describe("Python dependency bootstrap script", () => {
  it("uses bundled requirements, installs missing Python, and verifies docx", () => {
    expect(script).toContain("%~dp0requirements.txt");
    expect(script).toContain("python-3.13.14-amd64.exe");
    expect(script).toContain("InstallAllUsers=0 PrependPath=1 Include_pip=1");
    expect(script).toContain("-m pip install --user -r");
    expect(script).toContain("import docx");
    expect(script).toContain("Python313\\python.exe");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm exec vitest run tests/python-dependency-bootstrap.test.ts`

Expected: FAIL because `安装技术暗标检查依赖.bat` does not exist.

- [ ] **Step 3: Write the minimal batch script**

```bat
@echo off
setlocal EnableExtensions
chcp 65001 >nul
set "REQUIREMENTS=%~dp0requirements.txt"
set "PYTHON_INSTALLER=%TEMP%\python-3.13.14-amd64.exe"
set "PYTHON_URL=https://www.python.org/ftp/python/3.13.14/python-3.13.14-amd64.exe"

if not exist "%REQUIREMENTS%" goto requirements_missing
call :find_python
if defined PYTHON_CMD goto install_dependencies

powershell -NoProfile -ExecutionPolicy Bypass -Command "Invoke-WebRequest -UseBasicParsing -Uri '%PYTHON_URL%' -OutFile '%PYTHON_INSTALLER%'"
if errorlevel 1 goto download_failed
"%PYTHON_INSTALLER%" /quiet InstallAllUsers=0 PrependPath=1 Include_pip=1
if errorlevel 1 goto python_install_failed
call :find_python
if not defined PYTHON_CMD goto python_not_found

:install_dependencies
%PYTHON_CMD% -m ensurepip --upgrade
%PYTHON_CMD% -m pip install --user -r "%REQUIREMENTS%"
if errorlevel 1 goto dependency_failed
%PYTHON_CMD% -c "import docx"
if errorlevel 1 goto verification_failed
echo 技术暗标检查依赖安装完成。
goto end

:find_python
set "PYTHON_CMD="
python -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 1)" >nul 2>nul && set "PYTHON_CMD=python"
if defined PYTHON_CMD exit /b
py -3 -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 1)" >nul 2>nul && set "PYTHON_CMD=py -3"
if defined PYTHON_CMD exit /b
if exist "%LocalAppData%\Programs\Python\Python313\python.exe" set "PYTHON_CMD="%LocalAppData%\Programs\Python\Python313\python.exe""
exit /b

:download_failed
echo Python 安装包下载失败，请检查网络后重试。
goto end

:python_install_failed
echo Python 安装失败，请以当前 Windows 用户重新运行此脚本。
goto end

:python_not_found
echo 未能检测到 Python 3.8 或更高版本，请重新运行此脚本。
goto end

:requirements_missing
echo 未找到 requirements.txt。请从程序安装目录运行此脚本。
goto end

:dependency_failed
echo 依赖安装失败，请检查网络后重试。
goto end

:verification_failed
echo 依赖校验失败，请重新运行此脚本。

:end
pause
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm exec vitest run tests/python-dependency-bootstrap.test.ts`

Expected: PASS with one test passing.

- [ ] **Step 5: Commit the script and test**

```bash
git add src-tauri/resources/word-format-checker/安装技术暗标检查依赖.bat tests/python-dependency-bootstrap.test.ts
git commit -m "feat: add one-click Python dependency setup"
```

### Task 2: Replace manual dependency commands in the guide

**Files:**
- Modify: `投标文件智能审核-安装及使用指南.txt:41-78`
- Test: `tests/python-dependency-bootstrap.test.ts`

**Interfaces:**
- Consumes: `安装技术暗标检查依赖.bat` from Task 1.
- Produces: End-user instructions that only require locating and double-clicking the script.

- [ ] **Step 1: Extend the failing test for documentation guidance**

```ts
const guide = readFileSync(
  new URL("../投标文件智能审核-安装及使用指南.txt", import.meta.url),
  "utf8",
);

expect(guide).toContain("安装技术暗标检查依赖.bat");
expect(guide).toContain("双击运行");
expect(guide).not.toContain("python -m pip install --user -r");
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm exec vitest run tests/python-dependency-bootstrap.test.ts`

Expected: FAIL because the guide still asks the user to run pip manually.

- [ ] **Step 3: Update section four of the guide**

Replace the manual download, PATH, and pip command steps with these user-facing instructions:

```text
1. 打开程序安装目录中的：

   resources\word-format-checker\安装技术暗标检查依赖.bat

2. 双击运行该脚本，并在 Windows 提示时允许下载和安装 Python。

3. 等待窗口显示“技术暗标检查依赖安装完成”，再按任意键关闭窗口并启动本程序。

说明：脚本会自动检测已有的 Python；未安装时会下载 Python 3.13.14 64 位版本，并自动安装本程序需要的依赖。执行期间需要联网。
```

Update the Python-related common-question answer to direct users to run this script again.

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm exec vitest run tests/python-dependency-bootstrap.test.ts`

Expected: PASS with the script and guide checks passing.

- [ ] **Step 5: Run the frontend test suite**

Run: `npm test`

Expected: PASS with no TypeScript errors and all Vitest suites passing.

- [ ] **Step 6: Commit the documentation update**

```bash
git add 投标文件智能审核-安装及使用指南.txt tests/python-dependency-bootstrap.test.ts
git commit -m "docs: simplify Python setup instructions"
```
