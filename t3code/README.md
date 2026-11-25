<p align="center">
  <img src="./assets/prod/logo.svg" width="80" height="80" alt="VikEA" />
</p>

# VikEA

**Unofficial fork based on the [T3 Code source code](https://github.com/pingdotgg/t3code).** VikEA is maintained independently of T3 Tools, Inc.

VikEA runs local coding agents from a desktop interface. The Windows build includes a private Codex runtime and an isolated CC Switch gateway. Native Codex and gateway models appear independently in the same model selector.

## Windows build

Use the project workspace with the sibling Codex and CC Switch source directories. See the [project README](../README.md) for the toolchain and runtime build commands. Stage both runtime binaries from the workspace root with `./scripts/stage-runtime.ps1`, then run the desktop build from this directory.

```powershell
node scripts/build-desktop-artifact.ts --platform win --target nsis --arch x64 --build-version 0.1.17 --output-dir ../.local/dist-vikea
```

Configure the gateway under **Settings → Providers → CC Switch**. Sign in to native Codex under **Settings → Providers → Codex**. Each runtime uses its own state directory.

## Attribution and licenses

The [MIT license](./LICENSE) retains the original upstream copyright notice. See [branding and attribution](./TRADEMARK.md) and the license notices shipped with the bundled runtimes.
