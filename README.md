# VikEA

VikEA 是一个桌面编码代理工作区。Windows 版内置原生 Codex 和 CC Switch 网关，可以在同一个对话模型选择器中切换原生 Codex 与自定义 API 模型。

## 功能

- 使用原生 Codex、CC Switch 和其他受支持的编码代理，与项目代码进行多轮对话。界面提供项目、会话、终端、Git 工作树和差异视图。
- 从 CC Switch 预设或自定义表单添加 API 服务。网关支持 OpenAI Responses、OpenAI Chat Completions 和 Anthropic Messages，并提供连接测试、模型配置、用量和故障转移。
- 单独管理不同设备的网关配置。Windows 上的 VikEA 状态、原生 Codex 会话和网关数据使用独立目录，网关凭据由 Windows DPAPI 加密。

## 环境

| 用途 | 环境 |
| --- | --- |
| 桌面构建与运行 | Windows x64，PowerShell 7，Git，Visual Studio 2022 Build Tools 的 x64 C++ 工具链和 Windows SDK |
| 界面与打包 | Node.js 26.13.1、pnpm 12.14.0。Vite+ 由工作区依赖提供 |
| 运行时构建 | rustup。Codex 使用 Rust 1.99.1，CC Switch 使用 Rust 1.95，由各自的工具链文件选择 |

## 配置与构建流程

桌面界面连接本地服务，服务启动内置 Codex。选择 CC Switch 模型时，Codex 通过本机网关请求所配置的上游 API。三个源码目录是 `t3code/`（桌面、Web、移动端和服务端）、`cc-switch/`（API 网关）和 `codex/`（代理运行时）。

在加载了 MSVC x64 环境的 PowerShell 7 中，从仓库根目录执行以下命令。需要能下载依赖和构建工具。

1. 构建 Codex。

   ```powershell
   Push-Location ./codex/codex-rs
   rustup target add x86_64-pc-windows-msvc
   cargo build --locked --release --target x86_64-pc-windows-msvc -p codex-cli --bin codex
   Pop-Location
   ```

2. 构建 CC Switch 的无界面网关。

   ```powershell
   Push-Location ./cc-switch/src-tauri
   cargo build --locked --release --features sidecar-only --bin cc-switch-sidecar
   Pop-Location
   ```

3. 准备内置运行时并打包。

   ```powershell
   ./scripts/stage-runtime.ps1
   Push-Location ./t3code
   pnpm install --frozen-lockfile
   pnpm exec node scripts/build-desktop-artifact.ts --platform win --target nsis --arch x64 --build-version 0.1.17 --output-dir ../.local/dist-vikea
   Pop-Location
   ```

安装包生成在 `.local/dist-vikea/VikEA-0.1.17-x64.exe`。运行时准备脚本将两个可执行文件、许可和 SHA-256 清单写入 `.local/resources/`。

## 用法

1. **使用 CC Switch 模型。** 首次启动选择「Configure CC Switch」，也可以从「Settings → Providers → CC Switch」打开配置。选择预设或自定义供应商，根据 API 文档填写协议、Base URL、API Key 和模型 ID。保存、激活并测试连接，再从对话输入框的模型选择器选中该模型。
2. **使用原生 Codex。** 打开「Settings → Providers → Codex」，选择「Sign in with ChatGPT」，在隔离终端中确认运行登录命令并完成授权。远程设备使用设备代码授权。
3. **管理其他设备。** 打开「Settings → CC Switch」，先选择已连接且支持网关的设备，再编辑供应商、用量或故障转移设置。

| 配置项 | 用途 |
| --- | --- |
| `T3CCSW_HOME` | 覆盖默认的 Windows 数据目录 `%LOCALAPPDATA%\T3CCSW\t3-home` |
| `T3CCSW_PORT` | 指定本地桌面后端的端口 |
| `T3CCSW_RESOURCES` | 从源码运行时指定已准备的运行时资源目录 |

## 第三方组件与致谢

| 项目 | 用途与许可 |
| --- | --- |
| [T3 Code](https://github.com/pingdotgg/t3code) | 工作区界面和服务端基础，[MIT 许可](t3code/LICENSE) |
| [CC Switch](https://github.com/farion1231/cc-switch) | 供应商配置、协议转换和网关，[MIT 许可](cc-switch/LICENSE) |
| [OpenAI Codex](https://github.com/openai/codex) | 随 Windows 版内置的本地编码代理，[Apache-2.0 许可](codex/LICENSE)及 [NOTICE](codex/NOTICE) |

感谢三个上游项目及其贡献者。VikEA 是独立维护的非官方衍生项目，品牌归属见 [TRADEMARK.md](t3code/TRADEMARK.md)。
