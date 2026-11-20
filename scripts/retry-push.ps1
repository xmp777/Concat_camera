# 自动重推：每 5 分钟一次，直到 origin/main 同步（网络恢复后自动补推）
Set-Location G:\Projects\Concat_camera
for ($i = 1; $i -le 48; $i++) {
    $status = git status -sb | Select-Object -First 1
    if ($status -notmatch "ahead") {
        Write-Host "[$(Get-Date -Format HH:mm:ss)] synced: $status"
        exit 0
    }
    Write-Host "[$(Get-Date -Format HH:mm:ss)] attempt $i pushing..."
    git push origin main 2>&1 | Select-Object -Last 1
    Start-Sleep -Seconds 300
}
Write-Host "gave up after 48 attempts"
exit 1
