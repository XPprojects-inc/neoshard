# Runs one NeoShard component in a loop (started at boot by Task Scheduler, see install.ps1).
# Usage: run.ps1 -Name server|livekit|caddy
param([Parameter(Mandatory = $true)][ValidateSet('server', 'livekit', 'caddy')][string]$Name)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$cfg = Get-Content "$Root\config\settings.json" -Raw | ConvertFrom-Json
$sec = Get-Content "$Root\config\secrets.json" -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force "$Root\logs" | Out-Null
$log = "$Root\logs\$Name.log"

switch ($Name) {
    'server' {
        $env:DATABASE_URL = "postgres://neoshard:$($sec.db_password)@127.0.0.1:5432/neoshard"
        $env:NEOSHARD_BIND = '127.0.0.1:8080'
        $env:NEOSHARD_DOMAIN = $cfg.domain
        $env:NEOSHARD_PUBLIC_URL = "https://$($cfg.domain)"
        $env:NEOSHARD_SECRET_KEY = $sec.secret_key
        $env:NEOSHARD_TRUST_PROXY = 'true'
        $env:NEOSHARD_REGISTRATION_OPEN = 'true'
        $env:NEOSHARD_FILES_DIR = "$Root\data\files"
        $env:LIVEKIT_URL = "wss://$($cfg.domain):7880"
        $env:LIVEKIT_API_KEY = $sec.livekit_key
        $env:LIVEKIT_API_SECRET = $sec.livekit_secret
        $exe = "$Root\bin\neoshard-server.exe"; $argv = @()
    }
    'livekit' {
        $exe = "$Root\livekit\livekit-server.exe"; $argv = @('--config', "$Root\config\livekit.yaml")
    }
    'caddy' {
        # Certificates and state live next to the app, not in the SYSTEM profile.
        $env:XDG_DATA_HOME = "$Root\caddy\data"
        $env:XDG_CONFIG_HOME = "$Root\caddy\config"
        $exe = "$Root\caddy\caddy.exe"; $argv = @('run', '--config', "$Root\config\Caddyfile", '--adapter', 'caddyfile')
    }
}

# Native stderr lines must not stop the loop.
$ErrorActionPreference = 'Continue'
while ($true) {
    if ((Test-Path $log) -and (Get-Item $log).Length -gt 50MB) { Move-Item -Force $log "$log.old" }
    "=== $(Get-Date -Format s) starting $Name" | Out-File -Append -Encoding utf8 $log
    & $exe @argv 2>&1 | ForEach-Object { "$_" } | Out-File -Append -Encoding utf8 $log
    "=== $(Get-Date -Format s) $Name exited with $LASTEXITCODE, restarting in 5 s" | Out-File -Append -Encoding utf8 $log
    Start-Sleep 5
}
