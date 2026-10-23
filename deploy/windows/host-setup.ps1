# Host settings for a NeoShard server on Windows (run once, elevated, by the machine's owner):
# opens the inbound ports in Windows Firewall and keeps the PC from sleeping on mains power.
$ErrorActionPreference = 'Stop'
Get-NetFirewallRule -Group 'NeoShard' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
New-NetFirewallRule -Group 'NeoShard' -DisplayName 'NeoShard web and API (TCP 80, 443, 7880)' -Direction Inbound `
    -Protocol TCP -LocalPort 80, 443, 7880 -Action Allow -Profile Any | Out-Null
New-NetFirewallRule -Group 'NeoShard' -DisplayName 'NeoShard voice (TCP 7881)' -Direction Inbound `
    -Protocol TCP -LocalPort 7881 -Action Allow -Profile Any | Out-Null
New-NetFirewallRule -Group 'NeoShard' -DisplayName 'NeoShard voice (UDP 7882, 50000-50100)' -Direction Inbound `
    -Protocol UDP -LocalPort 7882, '50000-50100' -Action Allow -Profile Any | Out-Null
powercfg /change standby-timeout-ac 0
powercfg /change hibernate-timeout-ac 0
'firewall rules added, sleep on AC power disabled'
