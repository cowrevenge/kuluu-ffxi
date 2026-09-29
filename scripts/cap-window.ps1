# Capture a hidden/occluded kuluu window to PNG via
# PrintWindow(PW_RENDERFULLCONTENT). The window never needs to be on screen:
# KULUU_WINDOW_HIDDEN=1 runs render into the buried surface and this reads it back.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/cap-window.ps1 <process-name> <out.png> [wait-ms]
param(
    [Parameter(Mandatory=$true)][string]$Proc,
    [Parameter(Mandatory=$true)][string]$Out,
    [int]$WaitMs = 600
)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class W3 {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
'@
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
# Get-Process takes the bare image name; "kuluu.exe" is a wildcard pattern that matches nothing.
if ($Proc.EndsWith('.exe')) { $Proc = $Proc.Substring(0, $Proc.Length - 4) }
$w = Get-Process $Proc -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $w) { Write-Output "no window for process '$Proc'"; exit 1 }
# SWP_NOSIZE|SWP_NOMOVE|SWP_NOACTIVATE: no move, no focus steal.
[W3]::SetWindowPos($w.MainWindowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x13) | Out-Null
Start-Sleep -Milliseconds $WaitMs
$r = $w.MainWindowBounds
Write-Output "bounds=$($r.Width)x$($r.Height)"
if ($r.Width -le 0 -or $r.Height -le 0) { Write-Output 'zero-size window'; exit 1 }
$bmp = New-Object System.Drawing.Bitmap($r.Width, $r.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# PW_RENDERFULLCONTENT (2): renders the full window content even when hidden/occluded.
[W3]::PrintWindow($w.MainWindowHandle, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$bmp.Save($Out)
Write-Output "captured pid $($w.Id) title '$($w.MainWindowTitle)' -> $Out"
