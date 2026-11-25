$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$required = @(
    '.github/ISSUE_TEMPLATE/bug_report.yml',
    'docs/architecture.md',
    'LICENSE',
    'README.md',
    'package.json',
    'src/t3code/pnpm-lock.yaml',
    'src/t3code/LICENSE',
    'src/t3code/TRADEMARK.md',
    'src/cc-switch/src-tauri/Cargo.lock',
    'src/cc-switch/LICENSE',
    'src/codex/codex-rs/Cargo.lock',
    'src/codex/SOURCE_REVISION',
    'src/codex/LICENSE',
    'src/codex/NOTICE',
    'src/build/tier1-smoke.mjs',
    'src/build/tier23-smoke.py'
)
foreach ($path in $required) {
    if (!(Test-Path -LiteralPath (Join-Path $root $path) -PathType Leaf)) { throw "Missing $path" }
}

foreach ($name in @('build-windows.ps1', 'stage-runtime.ps1')) {
    $parseErrors = $null
    [System.Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot $name), [ref] $null, [ref] $parseErrors
    ) | Out-Null
    if ($parseErrors) { throw ($parseErrors -join "`n") }
}

foreach ($document in @('README.md', 'docs/architecture.md')) {
    $path = Join-Path $root $document
    $content = Get-Content -LiteralPath $path -Raw
    foreach ($link in [regex]::Matches($content, '\]\((?<target>[^)\s]+)\)')) {
        $target = $link.Groups['target'].Value.Split('#', 2)[0]
        if (!$target -or $target -match '^[a-zA-Z][a-zA-Z+.-]*:') { continue }
        if (!(Test-Path -LiteralPath (Join-Path (Split-Path -Parent $path) $target))) {
            throw "Broken local link in $document to $target"
        }
    }
}

Write-Output 'VikEA source layout, build scripts, and documentation links are valid.'
