#!/usr/bin/env pwsh
# ─────────────────────────────────────────────────────────────
#  start-all.ps1  –  ModelSentry local dev launcher
#
#  Usage:
#    .\start-all.ps1 [-VaultPassphrase <passphrase>]
#                    [-Config <path>]      (default: config/default.toml)
#                    [-ApiPort <port>]     (default: 7740)
#                    [-WebPort <port>]     (default: 5173)
#                    [-TimeoutSec <n>]     (default: 45)
#
#  The passphrase can also come from $env:MODELSENTRY_VAULT_PASSPHRASE.
#  On any failure the relevant log tail is printed inline so you can see
#  *what* broke without hunting through files.
# ─────────────────────────────────────────────────────────────
[CmdletBinding()]
param(
    [string] $VaultPassphrase = $env:MODELSENTRY_VAULT_PASSPHRASE,
    [string] $Config          = "config/default.toml",
    [int]    $ApiPort         = 7740,
    [int]    $WebPort         = 5173,
    [int]    $TimeoutSec      = 45
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── output helpers ─────────────────────────────────────────────
function Write-Header { Write-Host "`n$args" -ForegroundColor Cyan }
function Write-Ok     { Write-Host "  [OK]  $args" -ForegroundColor Green }
function Write-Warn   { Write-Host "  [!!]  $args" -ForegroundColor Yellow }
function Write-Fail   { Write-Host "  [XX]  $args" -ForegroundColor Red }
function Write-Info   { Write-Host "        $args" -ForegroundColor Gray }

# Print the last lines of a log file in red so errors are visible inline.
function Show-LogTail {
    param([string] $Path, [string] $Title, [int] $Lines = 25)
    Write-Host "  ┌─ $Title" -ForegroundColor DarkRed
    if (Test-Path $Path) {
        $tail = Get-Content -Path $Path -Tail $Lines -ErrorAction SilentlyContinue
        if ($tail) {
            $tail | ForEach-Object { Write-Host "  │ $_" -ForegroundColor Red }
        } else {
            Write-Host "  │ (log is empty — process may have died before writing)" -ForegroundColor DarkGray
        }
    } else {
        Write-Host "  │ (no log at $Path)" -ForegroundColor DarkGray
    }
    Write-Host "  └────────────────────────────────────────────────" -ForegroundColor DarkRed
}

# Return a description of whatever process is already listening on a port.
function Get-PortOwner {
    param([int] $Port)
    try {
        $conn = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop |
                Select-Object -First 1
        if ($conn) {
            $proc = Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue
            $name = if ($proc) { $proc.ProcessName } else { "unknown" }
            return "PID $($conn.OwningProcess) ($name)"
        }
    } catch { }
    return $null
}

# Wait for an HTTP endpoint, but bail out early if its process has died.
function Wait-Http {
    param(
        [string]                  $Label,
        [string]                  $Url,
        [System.Diagnostics.Process] $Proc,
        [int]                     $TimeoutSec = 45
    )
    Write-Info "Waiting for $Label at $Url …"
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        if ($Proc.HasExited) { return "process-exited" }
        try {
            $r = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
            if ($r.StatusCode -lt 500) { return "ok" }
        } catch { }
        Start-Sleep -Milliseconds 500
    }
    return "timeout"
}

$Root   = $PSScriptRoot
$WebDir = Join-Path $Root "web"
$LogDir = Join-Path $Root ".modelsentry"
$DaemonOut = Join-Path $LogDir "daemon.stdout.log"
$DaemonErr = Join-Path $LogDir "daemon.stderr.log"
$ViteOut   = Join-Path $LogDir "vite.stdout.log"
$ViteErr   = Join-Path $LogDir "vite.stderr.log"
$BuildLog  = Join-Path $LogDir "daemon.build.log"

# Log redirection requires the target directory to exist up front.
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null

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

$ConfigPath = if ([System.IO.Path]::IsPathRooted($Config)) { $Config } else { Join-Path $Root $Config }
if (-not (Test-Path $ConfigPath)) {
    Write-Fail "Config file not found: $ConfigPath"
    Write-Info "Copy the default and edit it:  Copy-Item config/default.toml config/local.toml"
    exit 1
}
Write-Ok "config found ($Config)"

if ([string]::IsNullOrEmpty($VaultPassphrase)) {
    Write-Warn "MODELSENTRY_VAULT_PASSPHRASE is not set."
    Write-Info "Set it inline and re-run:"
    Write-Info "  `$env:MODELSENTRY_VAULT_PASSPHRASE='your-passphrase'; .\start-all.ps1"
    Write-Info "Continuing — the daemon will refuse to start if a vault file already exists."
}

# Warn early if the ports are already taken (the most common silent failure).
foreach ($p in @(@{ N = "API"; Port = $ApiPort }, @{ N = "Web"; Port = $WebPort })) {
    $owner = Get-PortOwner -Port $p.Port
    if ($owner) {
        Write-Warn "$($p.N) port $($p.Port) is already in use by $owner — the new process may fail to bind."
    }
}

# ── install frontend deps if missing ───────────────────────────
if (-not (Test-Path (Join-Path $WebDir "node_modules"))) {
    Write-Header "Installing frontend dependencies (npm ci)…"
    Push-Location $WebDir
    try {
        npm ci 2>&1 | Tee-Object -FilePath (Join-Path $LogDir "npm-ci.log") | ForEach-Object { Write-Info $_ }
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "npm ci failed (exit $LASTEXITCODE)."
            Show-LogTail -Path (Join-Path $LogDir "npm-ci.log") -Title "npm ci output"
            exit 1
        }
    } finally { Pop-Location }
    Write-Ok "frontend dependencies installed"
}

# ── build daemon (dev profile) ─────────────────────────────────
Write-Header "Building modelsentry-daemon (dev)…"
Push-Location $Root
try {
    cargo build -p modelsentry-daemon 2>&1 | Tee-Object -FilePath $BuildLog | ForEach-Object { Write-Info $_ }
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "cargo build failed (exit $LASTEXITCODE). The compiler errors are above and in $BuildLog."
        Show-LogTail -Path $BuildLog -Title "cargo build errors" -Lines 40
        exit 1
    }
} finally { Pop-Location }
Write-Ok "daemon built"

# ── prepare environment ────────────────────────────────────────
if (-not [string]::IsNullOrEmpty($VaultPassphrase)) {
    $env:MODELSENTRY_VAULT_PASSPHRASE = $VaultPassphrase
}
if (-not $env:RUST_LOG) { $env:RUST_LOG = "info" }

$VaultFile = Join-Path $LogDir "vault"
if (Test-Path $VaultFile) {
    Write-Warn "Existing vault found at $VaultFile."
    Write-Info "If the passphrase is wrong, the daemon will exit immediately."
    Write-Info "To start fresh:  Remove-Item '$VaultFile'"
}

# ── start daemon ───────────────────────────────────────────────
Write-Header "Starting backend daemon…"
$DaemonExe = Join-Path $Root "target\debug\modelsentry-daemon.exe"

$DaemonProc = Start-Process -FilePath $DaemonExe `
    -ArgumentList @("--config", $Config) `
    -WorkingDirectory $Root `
    -PassThru `
    -RedirectStandardOutput $DaemonOut `
    -RedirectStandardError  $DaemonErr
Write-Info "Daemon PID $($DaemonProc.Id)  (logs → .modelsentry/daemon.*.log)"

# Give it a moment; a misconfigured daemon dies almost instantly.
Start-Sleep -Milliseconds 1500
if ($DaemonProc.HasExited) {
    Write-Fail "Daemon exited immediately (code $($DaemonProc.ExitCode)). Most likely: wrong vault passphrase, port $ApiPort in use, or a config error."
    Show-LogTail -Path $DaemonErr -Title "daemon stderr"
    Show-LogTail -Path $DaemonOut -Title "daemon stdout" -Lines 10
    exit 1
}

# ── start frontend ─────────────────────────────────────────────
Write-Header "Starting frontend dev server…"
$ViteProc = Start-Process -FilePath "cmd.exe" `
    -ArgumentList "/c", "npm run dev" `
    -WorkingDirectory $WebDir `
    -PassThru `
    -RedirectStandardOutput $ViteOut `
    -RedirectStandardError  $ViteErr
Write-Info "Vite PID $($ViteProc.Id)  (logs → .modelsentry/vite.*.log)"

Start-Sleep -Milliseconds 1500
if ($ViteProc.HasExited) {
    Write-Fail "Vite exited immediately (code $($ViteProc.ExitCode)). Most likely: port $WebPort in use or a missing dependency."
    Show-LogTail -Path $ViteErr -Title "vite stderr"
    Show-LogTail -Path $ViteOut -Title "vite stdout" -Lines 10
    Write-Warn "Stopping the daemon since the stack is incomplete."
    Stop-Process -Id $DaemonProc.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# ── health checks ──────────────────────────────────────────────
Write-Header "Health checks…"
$apiUrl = "http://127.0.0.1:$ApiPort/health"
$webUrl = "http://localhost:$WebPort"

$apiState = Wait-Http -Label "API daemon"      -Url $apiUrl -Proc $DaemonProc -TimeoutSec $TimeoutSec
$webState = Wait-Http -Label "Frontend (Vite)" -Url $webUrl -Proc $ViteProc   -TimeoutSec $TimeoutSec

if ($apiState -ne "ok") {
    Write-Fail "API daemon not reachable ($apiState)."
    Show-LogTail -Path $DaemonErr -Title "daemon stderr"
}
if ($webState -ne "ok") {
    Write-Fail "Frontend not reachable ($webState)."
    Show-LogTail -Path $ViteErr -Title "vite stderr"
}

# ── service summary ────────────────────────────────────────────
$bar = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
Write-Host ""
Write-Host $bar -ForegroundColor DarkCyan
Write-Host "  ModelSentry — service summary" -ForegroundColor Cyan
Write-Host $bar -ForegroundColor DarkCyan

if ($apiState -eq "ok") {
    Write-Host "  Backend  API    " -NoNewline -ForegroundColor White
    Write-Host "RUNNING" -ForegroundColor Green
} else {
    Write-Host "  Backend  API    " -NoNewline -ForegroundColor White
    Write-Host "DOWN — see daemon stderr above" -ForegroundColor Red
}
if ($webState -eq "ok") {
    Write-Host "  Frontend (Vite) " -NoNewline -ForegroundColor White
    Write-Host "RUNNING" -ForegroundColor Green
} else {
    Write-Host "  Frontend (Vite) " -NoNewline -ForegroundColor White
    Write-Host "DOWN — see vite stderr above" -ForegroundColor Red
}

Write-Host ""
Write-Host "  Open these URLs:" -ForegroundColor Cyan
Write-Host "    Dashboard (use this)   http://localhost:$WebPort"            -ForegroundColor White
Write-Host "    Daemon root / UI       http://127.0.0.1:$ApiPort/"           -ForegroundColor Gray
Write-Host "    Health check           http://127.0.0.1:$ApiPort/health"     -ForegroundColor Gray
Write-Host ""
Write-Host "  REST API (base http://127.0.0.1:$ApiPort/api):" -ForegroundColor Cyan
Write-Host "    GET    /api/probes                       list probes"        -ForegroundColor Gray
Write-Host "    POST   /api/probes                       create a probe"     -ForegroundColor Gray
Write-Host "    GET    /api/probes/{id}                  probe detail"       -ForegroundColor Gray
Write-Host "    POST   /api/probes/{id}/run-now          run a probe now"    -ForegroundColor Gray
Write-Host "    GET    /api/probes/{id}/runs             run history"        -ForegroundColor Gray
Write-Host "    GET    /api/probes/{id}/baselines/latest latest baseline"    -ForegroundColor Gray
Write-Host "    POST   /api/baselines/{id}               capture baseline"   -ForegroundColor Gray
Write-Host "    GET    /api/runs/{id}                     run detail"        -ForegroundColor Gray
Write-Host "    GET    /api/events                        alert events"      -ForegroundColor Gray
Write-Host "    POST   /api/events/{id}/acknowledge       ack an event"      -ForegroundColor Gray
Write-Host "    GET    /api/vault/keys                     list providers"   -ForegroundColor Gray
Write-Host "    PUT    /api/vault/keys/{provider}          set an API key"   -ForegroundColor Gray
Write-Host ""
Write-Host "  Logs:  .modelsentry/daemon.*.log   .modelsentry/vite.*.log"   -ForegroundColor DarkGray
Write-Host $bar -ForegroundColor DarkCyan
Write-Host ""
Write-Host "  Press Ctrl+C (or close this window) to stop both services." -ForegroundColor DarkYellow
Write-Host "  PIDs:  daemon=$($DaemonProc.Id)  vite=$($ViteProc.Id)" -ForegroundColor DarkGray
Write-Host ""

# ── monitor until a process exits, then surface its error ──────
try {
    while ($true) {
        if ($DaemonProc.HasExited) {
            Write-Fail "Daemon exited (code $($DaemonProc.ExitCode))."
            Show-LogTail -Path $DaemonErr -Title "daemon stderr"
            break
        }
        if ($ViteProc.HasExited) {
            Write-Fail "Vite exited (code $($ViteProc.ExitCode))."
            Show-LogTail -Path $ViteErr -Title "vite stderr"
            break
        }
        Start-Sleep -Seconds 3
    }
} finally {
    Write-Host "`nShutting down…" -ForegroundColor DarkYellow
    if (-not $DaemonProc.HasExited) {
        Stop-Process -Id $DaemonProc.Id -Force -ErrorAction SilentlyContinue
        Write-Info "Daemon stopped."
    }
    if (-not $ViteProc.HasExited) {
        Stop-Process -Id $ViteProc.Id -Force -ErrorAction SilentlyContinue
        Write-Info "Vite stopped."
    }
}
