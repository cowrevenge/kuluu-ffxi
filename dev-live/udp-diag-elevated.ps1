# ============================================================================
# udp-diag-elevated.ps1  —  run from an ELEVATED (Run as administrator) shell
# Self-contained, self-cleaning: leaves firewall exactly as it found it.
#
# Purpose: the one variable I couldn't test without admin rights.
#   Phase 1: allow-all inbound UDP rule (any profile), then fire a VM->PC UDP
#            packet at our listener and see if it arrives.
#   Phase 2 (only if phase 1 fails): firewall fully disabled for ~30s, retest,
#            re-enabled in finally{} no matter what.
# ============================================================================

$ErrorActionPreference = 'Stop'
$port = 39022

function Test-VmToPcUdp([string]$label) {
    $listener = New-Object Net.Sockets.UdpClient((New-Object Net.IPEndPoint([System.Net.IPAddress]::Any, $port)))
    $listener.Client.ReceiveTimeout = 15000
    # fire 10 packets from inside the Docker VM (host-net container) at this PC
    & docker exec cow-udp-probe python -c "import socket;s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);[s.sendto(b'diag',('192.168.40.231',39022)) for _ in range(10)];print('vm sent 10 packets')" | Out-Host
    try {
        $rpe = New-Object Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
        $bytes = $listener.Receive()
        Write-Host "[$label] SUCCESS - got: $([Text.Encoding]::ASCII.GetString($bytes))" -ForegroundColor Green
        return $true
    } catch {
        Write-Host "[$label] STILL LOST (no packet in 15s)" -ForegroundColor Red
        return $false
    } finally { $listener.Close() }
}

$rule = $null; $fwWasEnabled = $null
try {
    # ---------------- phase 1: allow-all inbound UDP, any profile ----------
    $rule = New-NetFirewallRule -DisplayName "bionic-udp-diag" -Direction Inbound `
             -Action Allow -Protocol UDP -Profile Any | Out-Null
    Write-Host "`n=== PHASE 1: allow-all inbound UDP rule (any profile) ===" 
    if (-not (Test-VmToPcUdp "phase1")) {
        # ---------------- phase 2: firewall fully off for the test ---------
        $fwWasEnabled = (Get-NetFirewallProfile | ForEach-Object { $_.Enabled }) -contains $true
        Set-NetFirewallProfile -Profile Domain,Private,Public -Enabled False
        Write-Host "`n=== PHASE 2: firewall disabled temporarily ==="
        Start-Sleep -Seconds 3   # let the state propagate
        Test-VmToPcUdp "phase2" | Out-Null
    }
}
finally {
    if ($rule) { Remove-NetFirewallRule -DisplayName "bionic-udp-diag" -ErrorAction SilentlyContinue }
    if ($null -ne $fwWasEnabled -and $fwWasEnabled) { Set-NetFirewallProfile -Profile Domain,Private,Public -Enabled True }
    Write-Host "`n[done] firewall state restored. summary:" 
    Get-NetFirewallProfile | Format-Table Name, Enabled
}
