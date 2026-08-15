# vite dev 常驻 runner（开发工作流）：崩溃自动重启。
# debug exe 从 devUrl（5199）加载前端——vite 必须活着，死了面板/宠物拿到的就是空气。
# 用法（后台常驻）：Invoke-CimMethod Win32_Process Create "powershell -NoProfile -File tools/run-vite-dev.ps1"
Set-Location $PSScriptRoot\..\app
while ($true) {
    npx vite --port 5199 --strictPort *>> "$env:TEMP\vite-5199.log"
    Start-Sleep 2
}
