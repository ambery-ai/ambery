# 定位 Terminal Overseer 所有窗口的位置和状态
param([switch]$Highlight)

Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class LocateWin {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumP lp, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder t, int m);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder c, int m);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumP(IntPtr h, IntPtr p);
    public struct RECT { public int L,T,R,B; }
}
'@

$ovPid = (Get-Process terminal-overseer -ErrorAction SilentlyContinue | Select-Object -First 1).Id
if (-not $ovPid) { Write-Host "terminal-overseer not running"; exit 1 }

$windows = @()
$cb = [LocateWin+EnumP]{ param($h,$_)
    $p = 0u; [LocateWin]::GetWindowThreadProcessId($h,[ref]$p)|Out-Null
    if ($p -ne $ovPid) { return $true }
    $c = New-Object System.Text.StringBuilder(256); [LocateWin]::GetClassName($h,$c,256)|Out-Null
    $t = New-Object System.Text.StringBuilder(256); [LocateWin]::GetWindowText($h,$t,256)|Out-Null
    $r = New-Object LocateWin+RECT; [LocateWin]::GetWindowRect($h,[ref]$r)|Out-Null
    if ($c.ToString() -eq 'Tauri Window') {
        $global:windows += [PSCustomObject]@{
            Name = $t.ToString()
            X = $r.L; Y = $r.T
            W = $r.R - $r.L; H = $r.B - $r.T
            Visible = [LocateWin]::IsWindowVisible($h)
            HWND = $h
        }
    }
    return $true
}
[LocateWin]::EnumWindows($cb,[IntPtr]::Zero)|Out-Null

$windows | Format-Table Name, X, Y, W, H, Visible -AutoSize
Write-Host "Total: $($windows.Count) window(s) | PID: $ovPid"

# ── Highlight：红框(可见)/绿框(隐藏) 边框标注 10 秒 ──
if ($Highlight) {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $forms = @()
    foreach ($w in $windows) {
        $color = if ($w.Visible) { 'Red' } else { 'Lime' }
        $f = New-Object System.Windows.Forms.Form
        $f.FormBorderStyle = 'None'
        $f.BackColor = [System.Drawing.Color]::Magenta
        $f.TransparencyKey = [System.Drawing.Color]::Magenta
        $f.ShowInTaskbar = $false
        $f.TopMost = $true
        $f.StartPosition = 'Manual'
        $f.Size = New-Object System.Drawing.Size($w.W, $w.H)
        $f.Location = New-Object System.Drawing.Point($w.X, $w.Y)
        $f.Tag = $color  # 每个窗独立存颜色
        $f.Add_Paint({
            param($sender, $e)
            $c = [System.Drawing.Color]::FromName($sender.Tag)
            $pen = New-Object System.Drawing.Pen($c, 2)
            $r = $sender.ClientRectangle
            $r.Width -= 1; $r.Height -= 1
            $e.Graphics.DrawRectangle($pen, $r)
            $pen.Dispose()
        })
        $f.Show()
        $forms += $f
    }
    Write-Host "红框=可见 绿框=隐藏，10 秒后消失…"
    Start-Sleep -Seconds 10
    foreach ($f in $forms) { $f.Close(); $f.Dispose() }
}
