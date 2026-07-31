# 交付复盘与避坑清单

## 这次最终状态

- 程序形态：Tauri 桌面应用，前端 TypeScript，后端 Rust。
- 输出形态：Markdown 结果文件，不再输出 PDF。
- 默认模型：DeepSeek `deepseek-v4-flash`。
- 暗标检查：随程序打包 `word-format-checker`，运行时依赖本机 Python。
- Windows 交付：NSIS 安装器。

## 下个程序优先避开的坑

1. 一开始就固定真实项目目录。
   本项目真实路径是 `2026-07-22/new-chat/app`，后续多次误入 `2026-07-27/new-chat`。下个项目开工第一步写清楚项目根目录。

2. 客户文档和开发文档分开。
   客户只需要安装、配置、使用、费用、安全。打包命令、缓存、NSIS、Cargo 放到 `docs/` 内部文档。

3. Windows 脚本保持 ASCII + CRLF。
   `.bat` 里不要写中文、emoji、特殊符号。中文系统上 `cmd` 可能在切换代码页前就解析脚本，导致乱码命令。

4. Windows 离线依赖提前设计。
   NSIS、WebView2、Python、pip 依赖都可能被网络限制卡住。交付前要准备离线说明或内置安装脚本。

5. 不要把 Python 工具误当成已完全内置。
   `word-format-checker` 可以随程序打包，但它仍需要 Python 运行环境。UI 上必须有“环境设置”和状态灯。

6. API Key 必须用系统凭据保存。
   不要写进配置文件、任务文件或审核结果。保存、清除、重启读取都要单独验收。

7. 模型名称和价格会变。
   DeepSeek 模型名、价格、接口规则都要以官方文档为准。默认值要集中配置，避免多处写死。

8. Markdown 表格要考虑预览效果。
   审核结果不要塞过宽表格。长证据、说明、建议更适合分段或列表；表格只放短字段。

9. 大文件和构建缓存不要拷贝。
   `node_modules`、`dist`、`src-tauri/target` 不进交付源码包。Windows 本机反复构建时再保留 `target`。

10. 每个问题都留一个回归测试。
    这次新增了批处理 ASCII 检查，能防止同类 Windows 乱码问题回潮。

## 标准收尾清单

交付前至少跑：

```bash
npm test
npm run build
cd src-tauri
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
git diff --check
```

Windows 上再验：

```bat
cd /d E:\app
npm install
npm test
npm run build
npm run tauri -- build
```

## 客户交付包应包含

- 安装程序 `.exe`。
- 客户版《投标文件智能审核-安装及使用指南.txt》。
- 测试用样例文件（如可提供）。
- DeepSeek API 获取和费用说明。

## 源码交接包应包含

- `app` 源码。
- `app/docs` 内部说明。
- `package-lock.json`、`Cargo.lock`。
- `src-tauri/resources/word-format-checker`。

源码交接包不应包含：

- `node_modules`
- `dist`
- `src-tauri/target`
- `.DS_Store`
- 真实 API Key
- 客户原始标书或审核报告
