# VikEA

VikEA 是一个桌面编码代理工作区，集成了 **T3 Code、CC Switch 和 OpenAI Codex**。T3 Code 提供工作区界面和本地服务，CC Switch 提供供应商配置与 API 协议网关，OpenAI Codex 作为内置编码代理运行。Windows 版可在同一个对话模型选择器中切换原生 Codex 与自定义 API 模型。

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

三个模块位于 [`src/t3code`](src/t3code)、[`src/cc-switch`](src/cc-switch) 和 [`src/codex`](src/codex)。项目级构建脚本位于 [`src/build`](src/build)。桌面界面连接本地服务。使用 CC Switch 模型时，Codex 通过本机网关请求所配置的上游 API。 [架构说明](docs/architecture.md)

在加载了 MSVC x64 环境的 PowerShell 7 中，从仓库根目录执行统一构建入口。首次构建需要下载依赖和构建工具。

```powershell
npm run build
```

构建会依次编译 Codex 和 CC Switch 网关、暂存运行时并生成校验清单、安装界面依赖以及生成 Windows NSIS 安装包。

| 命令 | 用途 |
| --- | --- |
| `npm run build` | 从源码构建运行时并打包 |
| `npm run package:windows` | 使用已有的 Codex 和 CC Switch Release 二进制打包 |
| `npm run stage:runtime` | 暂存二进制及许可文件，并生成 SHA-256 校验清单 |
| `npm run check:layout` | 检查模块布局、PowerShell 脚本语法和文档链接 |

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
| [T3 Code](https://github.com/pingdotgg/t3code) | 工作区界面和服务端基础，[MIT 许可](src/t3code/LICENSE) |
| [CC Switch](https://github.com/farion1231/cc-switch) | 供应商配置、协议转换和网关，[MIT 许可](src/cc-switch/LICENSE) |
| [OpenAI Codex](https://github.com/openai/codex) | 随 Windows 版内置的本地编码代理，[Apache-2.0 许可](src/codex/LICENSE)及 [NOTICE](src/codex/NOTICE) |

感谢三个上游项目及其贡献者。VikEA 是独立维护的非官方衍生项目。 [许可证说明](LICENSE) · [商标归属](src/t3code/TRADEMARK.md)

感谢 [Codex](https://github.com/codex) 协助实现和验证 VikEA 的 CC Switch 集成。
