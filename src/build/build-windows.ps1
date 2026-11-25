param(
    [switch] $UseExistingRuntimes,
    [switch] $KeepStage
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'VikEA currently provides a Windows x64 build from this entry point.' }

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$source = Join-Path $root 'src'
$metadata = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw | ConvertFrom-Json
$output = Join-Path $root '.local/dist-vikea'

function Invoke-BuildStep([string] $Directory, [string] $Command, [string[]] $Arguments) {
    Push-Location -LiteralPath $Directory
    try {
        & $Command @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$Command $($Arguments -join ' ') failed with exit code $LASTEXITCODE." }
    } finally {
        Pop-Location
    }
}

if (!$UseExistingRuntimes) {
    $codex = Join-Path $source 'codex/codex-rs'
    $target = 'x86_64-pc-windows-msvc'
    Invoke-BuildStep $codex 'rustup' @('target', 'add', $target)
    Invoke-BuildStep $codex 'cargo' @('build', '--locked', '--release', '--target', $target, '-p', 'codex-cli', '--bin', 'codex')
    Invoke-BuildStep (Join-Path $source 'cc-switch/src-tauri') 'cargo' @(
        'build', '--locked', '--release', '--features', 'sidecar-only', '--bin', 'cc-switch-sidecar'
    )
}

& (Join-Path $PSScriptRoot 'stage-runtime.ps1')
$desktop = Join-Path $source 't3code'
Invoke-BuildStep $desktop 'pnpm' @('install', '--frozen-lockfile')
$packaging = @(
    'exec', 'node', 'scripts/build-desktop-artifact.ts',
    '--platform', 'win', '--target', 'nsis', '--arch', 'x64',
    '--build-version', $metadata.version, '--output-dir', $output
)
if ($KeepStage) { $packaging += '--keep-stage' }
Invoke-BuildStep $desktop 'pnpm' $packaging
