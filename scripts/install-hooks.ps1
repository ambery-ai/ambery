# install-hooks.ps1：把 ambery-hook 挂进 ~/.claude/settings.json。
#   powershell -File scripts/install-hooks.ps1            # 安装（幂等，改前备份）
#   powershell -File scripts/install-hooks.ps1 -Uninstall # 卸载（只移除我们的条目）
param([switch]$Uninstall)

$ErrorActionPreference = "Stop"
$hooksDir = "$env:USERPROFILE\.claude\hooks"
$settingsPath = "$env:USERPROFILE\.claude\settings.json"
$scriptSrc = Join-Path $PSScriptRoot "ambery-hook.ps1"
$scriptDst = Join-Path $hooksDir "ambery-hook.ps1"
$marker = "ambery-hook.ps1"
$events = @("SessionStart", "UserPromptSubmit", "Stop", "SessionEnd", "Notification")

# 容错：settings.json 缺失不再是致命错误——
# 卸载语义 = 没有可卸载的东西；安装语义 = 从空对象起步（等价 Claude Code 无 hook 配置）
if (-not (Test-Path $settingsPath)) {
    if ($Uninstall) {
        Write-Host "uninstalled: settings.json 不存在（无可卸载条目）"
        exit 0
    }
    New-Item -ItemType Directory -Path (Split-Path $settingsPath -Parent) -Force | Out-Null
    "{}" | Out-File $settingsPath -Encoding UTF8
}

$settings = Get-Content $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
if (-not $settings.hooks) { $settings | Add-Member -NotePropertyName hooks -NotePropertyValue ([pscustomobject]@{}) }

if ($Uninstall) {
    foreach ($e in $events) {
        $arr = $settings.hooks.$e
        if (-not $arr) { continue }
        $kept = @($arr | Where-Object {
            $_.hooks -and @($_.hooks | Where-Object { $_.command -notlike "*$marker*" }).Count -gt 0
        })
        if ($kept.Count -eq 0) { $settings.hooks.PSObject.Properties.Remove($e) }
        else { $settings.hooks.$e = $kept }
    }
    $settings | ConvertTo-Json -Depth 20 | Out-File $settingsPath -Encoding UTF8
    Remove-Item $scriptDst -ErrorAction SilentlyContinue
    Write-Host "uninstalled: ambery hook entries removed, script deleted"
    exit 0
}

# install：备份 → 拷脚本 → 追加条目（幂等：已存在则跳过）
$bak = "$settingsPath.bak-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
Copy-Item $settingsPath $bak
New-Item -ItemType Directory -Path $hooksDir -Force | Out-Null
Copy-Item $scriptSrc $scriptDst -Force

foreach ($e in $events) {
    if (-not $settings.hooks.$e) {
        $settings.hooks | Add-Member -NotePropertyName $e -NotePropertyValue @()
    }
    $exists = @($settings.hooks.$e | Where-Object {
        $_.hooks -and @($_.hooks | Where-Object { $_.command -like "*$marker*" }).Count -gt 0
    }).Count -gt 0
    if (-not $exists) {
        $entry = [pscustomobject]@{
            hooks = @([pscustomobject]@{
                type = "command"
                command = "powershell -NoProfile -File $scriptDst"
                shell = "powershell"
                timeout = 5
                async = $true
            })
        }
        $settings.hooks.$e = @($settings.hooks.$e) + $entry
        Write-Host "+ $e"
    } else {
        Write-Host "= $e (已存在,跳过)"
    }
}
$settings | ConvertTo-Json -Depth 20 | Out-File $settingsPath -Encoding UTF8
Write-Host "installed: backup at $bak"
Write-Host "提示: WT 开启「在所有桌面上显示此应用的窗口」可让其他桌面的实例也可读（可选）"
