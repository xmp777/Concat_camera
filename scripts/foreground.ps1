# 把 Concat 主窗口带到前台并最大化（SOP §12.4 UI 验收用）
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class WinFg {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@
$p = Get-Process concat -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($p) {
    Write-Host "window: $($p.MainWindowTitle)"
    [WinFg]::ShowWindow($p.MainWindowHandle, 3) | Out-Null
    [WinFg]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
    Write-Host "foregrounded"
} else {
    Write-Host "no concat window"
}
