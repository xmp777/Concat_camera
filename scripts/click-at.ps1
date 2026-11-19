# 把鼠标移到 (X,Y) 并左键单击（SOP §12.4 UI 验收用）
param([int]$X, [int]$Y)
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Mouse {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
    public const uint LEFTDOWN = 0x02; public const uint LEFTUP = 0x04;
}
"@
[Mouse]::SetCursorPos($X, $Y) | Out-Null
Start-Sleep -Milliseconds 120
[Mouse]::mouse_event([Mouse]::LEFTDOWN, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Mouse]::mouse_event([Mouse]::LEFTUP, 0, 0, 0, [UIntPtr]::Zero)
Write-Host "clicked $X,$Y"
