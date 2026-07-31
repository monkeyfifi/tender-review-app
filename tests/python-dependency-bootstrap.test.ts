/// <reference types="vite/client" />

import { describe, expect, it } from "vitest";
import script from "../src-tauri/resources/word-format-checker/安装技术暗标检查依赖.bat?raw";
import guide from "../投标文件智能审核-安装及使用指南.txt?raw";

describe("Python dependency bootstrap script", () => {
  it("uses bundled requirements, installs missing Python, and verifies docx", () => {
    expect(script).toContain("%~dp0requirements.txt");
    expect(script).toContain("python-3.13.14-amd64.exe");
    expect(script).toContain("InstallAllUsers=0 PrependPath=1 Include_pip=1");
    expect(script).toContain("-m pip install --user -r");
    expect(script).toContain("import docx");
    expect(script).toContain("Python313\\python.exe");
  });

  it("tells users to run the one-click script instead of pip manually", () => {
    expect(guide).toContain("安装技术暗标检查依赖.bat");
    expect(guide).toContain("双击运行");
    expect(guide).not.toContain("python -m pip install --user -r");
  });
});
