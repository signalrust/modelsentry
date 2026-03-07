#!/usr/bin/env pwsh
# ─────────────────────────────────────────────────────────────
#  start-all.ps1  –  ModelSentry local dev launcher
#  Usage:  .\start-all.ps1 [-VaultPassphrase <passphrase>]
#                          [-Config <path>]
#                          [-ApiPort <port>]
#                          [-WebPort <port>]
# ─────────────────────────────────────────────────────────────
[CmdletBinding()]
param(
    # Vault passphrase – also honoured via $env:MODELSENTRY_VAULT_PASSPHRASE
    [string] $VaultPassphrase = $env:MODELSENTRY_VAULT_PASSPHRASE,

    # Path to daemon TOML config  (default: config/default.toml)
    [string] $Config = "config/default.toml",

    # Expected daemon API port (used only for health-check URL)
    [int]    $ApiPort = 7740,

    # Expected Vite dev-server port (used only for health-check URL)
    [int]    $WebPort = 5173
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── helpers ────────────────────────────────────────────────────
function Write-Header { Write-Host "`n$args" -ForegroundColor Cyan }
function Write-Ok     { Write-Host "  [OK]  $args" -ForegroundColor Green }
function Write-Warn   { Write-Host "  [!!]  $args" -ForegroundColor Yellow }
function Write-Fail   { Write-Host "  [!!]  $args" -ForegroundColor Red }
function Write-Info   { Write-Host "        $args" -ForegroundColor Gray }

$Root = $PSScriptRoot

# ── preflight checks ───────────────────────────────────────────
Write-Header "ModelSentry — preflight checks"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Fail "cargo not found. Install Rust: https://rustup.rs"
    exit 1
}
Write-Ok "cargo found  ($(cargo --version))"

if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    Write-Fail "npm not found. Install Node.js: https://nodejs.org"
    exit 1
}
Write-Ok "npm found    ($(npm --version))"

if ([string]::IsNullOrEmpty($VaultPassphrase)) {
    Write-Warn "MODELSENTRY_VAULT_PASSPHRASE is not set."
    Write-Info "Either export it in your shell, or re-run:"
    Write-Info "  \$env:MODELSENTRY_VAULT_PASSPHRASE='your-passphrase' ; .\start-all.ps1"
    Write-Info "Starting anyway – daemon will fail if a vault file already exists."
}

# ── check/install frontend deps ────────────────────────────────
$WebDir = Join-Path $Root "web"
if (-not (Test-Path (Join-Path $WebDir "node_modules"))) {
    Write-Header "Installing frontend dependencies (npm ci)…"
    Push-Location $WebDir
    npm ci --silent
    Pop-Location
    Write-Ok "npm ci complete"
}

# ── build daemon (dev profile) ─────────────────────────────────
Write-Header "Building modelsentry-daemon (dev)…"
Push-Location $Root
cargo build -p modelsentry-daemon -q 2>&1 | ForEach-Object { Write-Info $_ }
if ($LASTEXITCODE -ne 0) {
    Write-Fail "cargo build failed. Fix the errors above and try again."
    exit 1
}
Write-Ok "daemon built"
Pop-Location

# ── start daemon ───────────────────────────────────────────────
Write-Header "Starting backend daemon…"

$DaemonExe  = Join-Path $Root "target\debug\modelsentry-daemon.exe"
$DaemonArgs = @("--config", $Config)

# Set env vars on THIS process so child processes inherit them automatically.
if (-not [string]::IsNullOrEmpty($VaultPassphrase)) {
    $env:MODELSENTRY_VAULT_PASSPHRASE = $VaultPassphrase
}
if (-not $env:RUST_LOG) { $env:RUST_LOG = "info" }

$VaultFile = Join-Path $Root ".modelsentry\vault"
if ((Test-Path $VaultFile)) {
    Write-Warn "Existing vault found at $VaultFile."
    Write-Info "If the passphrase is wrong the daemon will fail. Delete it to start fresh:"
    Write-Info "  Remove-Item $VaultFile"
}

$DaemonProc = Start-Process -FilePath $DaemonExe `
    -ArgumentList $DaemonArgs `
    -WorkingDirectory $Root `
    -PassThru `
    -RedirectStandardOutput (Join-Path $Root ".modelsentry\daemon.stdout.log") `
    -RedirectStandardError  (Join-Path $Root ".modelsentry\daemon.stderr.log")

Write-Info "Daemon PID: $($DaemonProc.Id)  (logs → .modelsentry/daemon.*.log)"

# ── start frontend ─────────────────────────────────────────────
Write-Header "Starting frontend dev server…"

$ViteProc = Start-Process -FilePath "cmd.exe" `
    -ArgumentList "/c", "npm run dev" `
    -WorkingDirectory $WebDir `
    -PassThru `
    -RedirectStandardOutput (Join-Path $Root ".modelsentry\vite.stdout.log") `
    -RedirectStandardError  (Join-Path $Root ".modelsentry\vite.stderr.log")

Write-Info "Vite PID: $($ViteProc.Id)  (logs → .modelsentry/vite.*.log)"

# ── wait for services to become reachable ──────────────────────
function Wait-Http {
    param([string]$Label, [string]$Url, [int]$TimeoutSec = 30)
    Write-Info "Waiting for $Label at $Url …"
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
            if ($r.StatusCode -lt 500) { return $true }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

Write-Header "Health checks…"

$apiUrl = "http://127.0.0.1:$ApiPort/health"
$webUrl = "http://localhost:$WebPort"

$apiOk = Wait-Http -Label "API daemon"       -Url $apiUrl -TimeoutSec 30
$webOk = Wait-Http -Label "Frontend (Vite)"  -Url $webUrl -TimeoutSec 30

# ── summary ────────────────────────────────────────────────────
Write-Host ""
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor DarkCyan
Write-Host "  ModelSentry — service summary" -ForegroundColor Cyan
Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor DarkCyan

if ($apiOk) {
    Write-Host "  Backend  API   " -NoNewline -ForegroundColor White
    Write-Host "RUNNING " -NoNewline -ForegroundColor Green
    Write-Host "→  http://127.0.0.1:$ApiPort" -ForegroundColor Cyan
    Write-Host "  API Docs / Health    →  http://127.0.0.1:$ApiPort/health" -ForegroundColor Gray
} else {
    Write-Host "  Backend  API   " -NoNewline -ForegroundColor White
    Write-Host "NOT READY" -ForegroundColor Red
    Write-Host "    (check .modelsentry/daemon.stderr.log)" -ForegroundColor DarkRed
}

if ($webOk) {
    Write-Host "  Frontend (Vite)" -NoNewline -ForegroundColor White
    Write-Host " RUNNING " -NoNewline -ForegroundColor Green
    Write-Host "→  http://localhost:$WebPort" -ForegroundColor Cyan
} else {
    Write-Host "  Frontend (Vite)" -NoNewline -ForegroundColor White
    Write-Host " NOT READY" -ForegroundColor Red
    Write-Host "    (check .modelsentry/vite.stderr.log)" -ForegroundColor DarkRed
}

Write-Host "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" -ForegroundColor DarkCyan
Write-Host ""
Write-Host "  Press Ctrl+C or close this window to stop monitoring." -ForegroundColor DarkYellow
Write-Host "  PIDs:  daemon=$($DaemonProc.Id)  vite=$($ViteProc.Id)" -ForegroundColor DarkGray
Write-Host ""

# ── keep running until both child processes exit ───────────────
try {
    while ($true) {
        $dStopped = $DaemonProc.HasExited
        $vStopped = $ViteProc.HasExited

        if ($dStopped) {
            Write-Warn "Daemon exited (code $($DaemonProc.ExitCode)). Check .modelsentry/daemon.stderr.log"
        }
        if ($vStopped) {
            Write-Warn "Vite exited (code $($ViteProc.ExitCode)). Check .modelsentry/vite.stderr.log"
        }
        if ($dStopped -or $vStopped) { break }

        Start-Sleep -Seconds 5
    }
} finally {
    Write-Host "`nShutting down…" -ForegroundColor DarkYellow
    if (-not $DaemonProc.HasExited) { Stop-Process -Id $DaemonProc.Id -Force -ErrorAction SilentlyContinue; Write-Info "Daemon stopped." }
    if (-not $ViteProc.HasExited)   { Stop-Process -Id $ViteProc.Id   -Force -ErrorAction SilentlyContinue; Write-Info "Vite stopped."   }
}
