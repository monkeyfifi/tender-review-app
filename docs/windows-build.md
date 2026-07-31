# Windows 构建与交付

## 环境要求

- Windows 10 / Windows 11 x64。
- Node.js LTS。
- Rust stable MSVC 工具链。
- Microsoft C++ Build Tools，安装 `Desktop development with C++`。
- Microsoft Edge WebView2 Runtime。最终用户机器缺失时，安装器会提示安装。

## 源码拷贝

从 Mac 或其他电脑拷贝到 Windows 测试机时，只拷贝 `app` 源码即可。不要拷贝：

- `node_modules`
- `dist`
- `src-tauri/target`
- `.git`

推荐放到：

```bat
E:\app
```

## 常用验证命令

在 Windows 命令行进入 `E:\app`：

```bat
cd /d E:\app
npm install
npm test
npm run build
```

Rust 后端验证：

```bat
cd /d E:\app\src-tauri
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
```

## 开发运行

仅在需要打开开发版窗口调试时执行：

```bat
cd /d E:\app
npm run tauri -- dev
```

`dev` 不是打安装包前置步骤。窗口启动后命令行显示 `Running target\debug\app.exe` 属于正常状态，关闭程序窗口后命令才会结束。

## 打安装包

```bat
cd /d E:\app
npm run tauri -- build
```

生成文件通常位于：

```text
E:\app\src-tauri\target\release\bundle\nsis
```

或自定义 `CARGO_TARGET_DIR` 指向的目录下。

## NSIS 离线处理

如果 Windows 无法访问 GitHub，Tauri 可能无法自动下载 NSIS。可手动准备：

```text
%LOCALAPPDATA%\tauri\nsis-3.11
```

展开后应能看到：

```text
%LOCALAPPDATA%\tauri\nsis-3.11\makensis.exe
%LOCALAPPDATA%\tauri\nsis-3.11\Bin\makensis.exe
%LOCALAPPDATA%\tauri\nsis-3.11\Include
%LOCALAPPDATA%\tauri\nsis-3.11\Plugins
%LOCALAPPDATA%\tauri\nsis-3.11\Stubs
```

注意不要多套一层 `nsis-3.11\nsis-3.11`。

## Python 环境

`word-format-checker` 已随程序资源打包，但技术暗标检查仍需要本机 Python 和依赖。程序右上角“环境设置”可打开安装脚本：

```text
resources\word-format-checker\setup-word-format-checker.bat
```

脚本必须保持 ASCII + CRLF，避免 Windows `cmd` 在中文系统下解析 UTF-8 批处理出现乱码命令。

## 缓存清理原则

正常反复测试时，不需要删除：

```text
E:\app\src-tauri\target
```

它是 Rust 编译缓存，保留可显著加快后续构建。只有源码转移、磁盘不足或缓存异常时再删除。

交付源码包前建议删除：

```text
node_modules
dist
src-tauri\target
```

## Windows 验收重点

- 安装包可生成并安装。
- 首次启动正常，WebView2 正常加载。
- API Key 保存后，重启软件仍可使用。
- 未配置 API 时不应显示“模型已配置”。
- 未安装 Python 时，“环境设置”可打开安装脚本。
- Python 依赖安装后，环境绿灯变为“环境已就绪”。
- 1 份招标文件 + 多份投标文件 + 多份技术暗标可完整跑通。
- 审核完成后可预览 Markdown，并可打开结果目录。
