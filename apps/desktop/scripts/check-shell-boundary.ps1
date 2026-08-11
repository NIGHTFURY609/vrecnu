<#
.SYNOPSIS
    Fails if a GUI framework has leaked out of vrecnu-shell-tauri.

.DESCRIPTION
    ADR-006 reversed ADR-001 and allowed Tauri back, on the condition that the shell stays
    replaceable. That condition is worth nothing as a convention and everything as a check,
    so this runs in CI on every commit.

    Two independent checks, because either alone can be fooled:

      1. "cargo tree" on every shell-free crate. If tauri (or tao, wry, webview2-com) appears
         anywhere in one of those graphs, the boundary is gone.
      2. "cargo build -p vrecnu-cli". A link-time proof rather than a text search: the shell
         is only genuinely swappable if a shell-free binary actually links.

    If this fails, read read_this_idiot.md before "fixing" it by deleting the check.

    NOTE: ASCII only. Windows PowerShell 5.1 reads a UTF-8 file without a BOM as ANSI, and a
    stray non-ASCII byte turns into a parser error rather than a mangled comment.
#>
[CmdletBinding()]
param(
    [string]$ManifestPath = "$PSScriptRoot\..\Cargo.toml",
    [switch]$SkipBuild
)

# Deliberately NOT 'Stop'. Cargo writes its ordinary progress ("Compiling foo v1.0") to
# stderr, and Windows PowerShell 5.1 turns a native command's stderr into NativeCommandError
# records. Under 'Stop' that makes a perfectly successful `cargo build` look like a failure.
# Success is judged by $LASTEXITCODE, explicitly, everywhere below.
$ErrorActionPreference = 'Continue'
$forbidden = @('tauri', 'tao', 'wry', 'webview2-com')
$shellFreeCrates = @('vrecnu-cli', 'vrecnu-engine', 'vrecnu-capture', 'vrecnu-audio',
                     'vrecnu-camera', 'vrecnu-encode', 'vrecnu-publish-server',
                     'vrecnu-bubble-win32', 'vrecnu-shell-api', 'vrecnu-types')

$failed = $false

foreach ($crate in $shellFreeCrates) {
    Write-Host "checking $crate ..." -NoNewline
    $tree = cargo tree --manifest-path $ManifestPath -p $crate --edges normal --prefix none 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) {
        Write-Host " ERROR"
        Write-Host $tree
        exit 1
    }

    $hits = @()
    foreach ($name in $forbidden) {
        # Anchor on "name vX" at a line start, so a substring such as "tauri-plugin-x" inside
        # an unrelated package name cannot produce a false positive.
        if ($tree -match "(?m)^$([regex]::Escape($name))\s+v\d") {
            $hits += $name
        }
    }

    if ($hits.Count -gt 0) {
        Write-Host " FAIL"
        Write-Host "  $crate depends on: $($hits -join ', ')"
        $failed = $true
    } else {
        Write-Host " ok"
    }
}

if ($failed) {
    Write-Host ""
    Write-Host "A GUI framework has leaked past the shell boundary (ADR-006)." -ForegroundColor Red
    Write-Host "Every crate above must build without one; only vrecnu-shell-tauri may name Tauri."
    Write-Host "See read_this_idiot.md for what this protects and why it matters."
    exit 1
}

if ($SkipBuild) {
    Write-Host ""
    Write-Host "Dependency-graph check passed. Link-time proof skipped (-SkipBuild)."
    exit 0
}

Write-Host ""
Write-Host "Link-time proof: building vrecnu-cli ..."
cargo build --manifest-path $ManifestPath -p vrecnu-cli
if ($LASTEXITCODE -ne 0) {
    Write-Host "vrecnu-cli failed to build, so the shell-free path is broken." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Shell boundary intact: the engine links with no GUI framework present." -ForegroundColor Green
