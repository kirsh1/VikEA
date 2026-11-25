param(
    [string] $ResourcesPath = (Join-Path $PSScriptRoot '../../.local/resources')
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$assets = [ordered]@{
    'codex/codex.exe' = 'codex/codex-rs/target/x86_64-pc-windows-msvc/release/codex.exe'
    'cc-switch/cc-switch-sidecar.exe' = 'cc-switch/src-tauri/target/release/cc-switch-sidecar.exe'
}
foreach ($relative in $assets.Keys) {
    $source = Join-Path $workspace $assets[$relative]
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing runtime build $source" }
}

$ResourcesPath = [IO.Path]::GetFullPath($ResourcesPath)
$manifest = [ordered]@{}
foreach ($relative in $assets.Keys) {
    $destination = Join-Path $ResourcesPath $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspace $assets[$relative]) -Destination $destination -Force
    $manifest[$relative] = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
}

Copy-Item -LiteralPath (Join-Path $workspace 'codex/LICENSE') -Destination (Join-Path $ResourcesPath 'codex/LICENSE.txt') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'codex/NOTICE') -Destination (Join-Path $ResourcesPath 'codex/NOTICE.txt') -Force
Copy-Item -LiteralPath (Join-Path $workspace 'cc-switch/LICENSE') -Destination (Join-Path $ResourcesPath 'cc-switch/LICENSE.txt') -Force

$revision = (Get-Content -LiteralPath (Join-Path $workspace 'codex/SOURCE_REVISION') -Raw).Trim()
if ($revision -notmatch '^[0-9a-f]{40}$') { throw 'Cannot identify the Codex snapshot' }
[ordered]@{ layoutVersion = 1; entrypoint = 'codex.exe'; version = '0.0.0'; sourceRevision = $revision } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $ResourcesPath 'codex/codex-package.json') -Encoding utf8NoBOM
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $ResourcesPath 'runtime-sha256.json') -Encoding utf8NoBOM
Write-Output $ResourcesPath
