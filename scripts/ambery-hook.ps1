# ambery-hook.ps1：Claude Code command hook → ambery。
# stdin 读 payload JSON；SessionStart/UserPromptSubmit 向 stdout 输出 sessionTitle（定位标记）；
# 所有事件 fire-and-forget POST 到 127.0.0.1:47600/hook（失败静默，绝不阻塞 Claude）。
# 隐私边界：只转发 session_id/cwd/kind/prompt/message/last_assistant_message，不读 transcript。
$ErrorActionPreference = "Stop"
$logFile = "$env:USERPROFILE\.claude\hooks\ambery-errors.log"

function sid8([string]$sid) { if ($sid.Length -ge 8) { $sid.Substring(0, 8) } else { $sid } }
function projectOf([string]$cwd) {
    if ([string]::IsNullOrWhiteSpace($cwd)) { return "unknown" }
    return Split-Path -Leaf $cwd.TrimEnd('\', '/')
}
function postHook([hashtable]$body) {
    try {
        $json = $body | ConvertTo-Json -Compress
        # 临时文件传 body：PS5.1 管道会给 UTF8 加 BOM（server 400 的根因）；PS 原生命令行又吃内层引号
        $tmp = [IO.Path]::Combine($env:TEMP, "ambery-hook-$PID.json")
        [IO.File]::WriteAllText($tmp, $json, (New-Object System.Text.UTF8Encoding($false)))
        curl.exe -s -m 3 -X POST http://127.0.0.1:47600/hook -H "Content-Type: application/json" -d "@$tmp" | Out-Null
        Remove-Item $tmp -ErrorAction SilentlyContinue
    } catch {
        "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') | postHook | $($_.Exception.Message)" | Out-File -Append -Encoding UTF8 $logFile
    }
}

try {
    [Console]::InputEncoding = [System.Text.Encoding]::UTF8
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8  # sessionTitle 含 ·,PS5.1 默认 ANSI 会毁掉 marker
    $raw = [Console]::In.ReadToEnd()
    if (-not $raw) { exit 0 }
    $j = $raw | ConvertFrom-Json

    $event = $j.hook_event_name
    $sid = [string]$j.session_id
    $cwd = [string]$j.cwd
    $sid8 = sid8 $sid
    $project = projectOf $cwd
    $marker = "$project·$sid8"

    switch ($event) {
        "SessionStart" {
            # 定位标记（marker 前缀不变量）
            @{ sessionTitle = $marker } | ConvertTo-Json -Compress | Write-Output
            postHook @{ event = "session_start"; session_id = $sid; cwd = $cwd; kind = "claude" }
        }
        "UserPromptSubmit" {
            $prompt = [string]$j.prompt
            $short = if ($prompt.Length -gt 20) { $prompt.Substring(0, 20) } else { $prompt }
            # 必须重发（claude 会按 prompt 自动命名，marker 不自发重申会被冲掉）
            @{ sessionTitle = "$marker | $short" } | ConvertTo-Json -Compress | Write-Output
            postHook @{ event = "user_prompt"; session_id = $sid; cwd = $cwd; kind = "claude"; prompt = $prompt }
        }
        "Stop" {
            postHook @{ event = "stop"; session_id = $sid; cwd = $cwd; kind = "claude"; last_assistant_message = [string]$j.last_assistant_message }
        }
        "SessionEnd" {
            postHook @{ event = "session_end"; session_id = $sid; cwd = $cwd; kind = "claude" }
        }
        "Notification" {
            postHook @{ event = "notification"; session_id = $sid; cwd = $cwd; kind = "claude"; message = [string]$j.message }
        }
    }
} catch {
    "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') | $($_.Exception.Message)" | Out-File -Append -Encoding UTF8 $logFile
}
exit 0
