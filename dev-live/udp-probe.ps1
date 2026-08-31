# UDP reachability test from this PC against the live Docker stack.
#   19475/udp = cow-udp-probe (echo container; proves full published-UDP round-trip)
#   54230/udp = cow-map       (gameplay port; send-only here, delivery confirmed server-side)
# Usage: powershell -NoProfile -File C:\Cow_Kuluu_ffxi-engine\dev-live\udp-probe.ps1
$targets = @("127.0.0.1") +
  (Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.PrefixOrigin -in "Dhcp","Manual" } | Select-Object -ExpandProperty IPAddress)

function Test-UdpPort([string]$ip, [int]$port, [switch]$expectEcho) {
  $c = New-Object System.Net.Sockets.UdpClient
  $c.Client.ReceiveTimeout = 2500
  $payload = "cowprobe-" + [guid]::NewGuid().ToString("N").Substring(0,8)
  try {
    [void]$c.Send([System.Text.Encoding]::ASCII.GetBytes($payload), $payload.Length, $ip, $port)
    if ($expectEcho) {
      try {
        $ep = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
        $resp = [System.Text.Encoding]::ASCII.GetString($c.Receive().ToArray())
        if ($resp -match "ECHO:" + $payload) { return "PASS (echo received: $resp)" } else { return "WEIRD REPLY: $resp" }
      } catch {
        return "FAIL sent ok, no reply within 2.5s   <-- published-UDP return path dead"
      }
    }
    return "SENT ($payload) - delivery checked server-side"
  } finally { $c.Close() }
}

foreach ($ip in $targets) {
  Write-Host "" + ("=== target " + $ip + " ===")
  Write-Host ("  19475/udp (probe, expect echo): " + (Test-UdpPort $ip 19475 -expectEcho))
  if ($ip -eq "127.0.0.1") { continue }
  Write-Host ("  54230/udp (map, send-only):     " + (Test-UdpPort $ip 54230))
}
