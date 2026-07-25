# Terminal Overseer 开发验证函数库
# 用法（点源加载后调用）：
#   . .\scripts\dev-tools.ps1
#   Start-OverseerDev          # 起 vite dev + Tauri 壳（带 storage/sidecar 环境）
#   Get-OverseerWindowText     # UIA 读 webview 里的文本（验证渲染）
#   Test-OverlayTransparency   # 截屏采样验证 overlay 透明
#   Stop-OverseerDev           # 全部停掉

$script:OverseerRepo = "D:\Project\bloominginthemud\terminal-overseer"

function Start-OverseerDev {
    param([switch]$NoSidecar, [switch]$NoVite, [string]$StorageDir, [string]$ConfigDir)

    if (-not $NoVite) {
        $viteUp = $false
        try { Invoke-WebRequest -Uri "http://localhost:5173" -UseBasicParsing -TimeoutSec 2 | Out-Null; $viteUp = $true } catch {}
        if (-not $viteUp) {
            Start-Process -FilePath "cmd.exe" -ArgumentList '/c npx vite --port 5173 --strictPort > vite-dev.log 2>&1' `
                -WorkingDirectory "$script:OverseerRepo\app" -WindowStyle Hidden
            Start-Sleep -Seconds 4
        }
    }

    # Config/Storage 默认走 %USERPROFILE%\.config\terminal-overseer（core/paths.rs）；
    # 需要隔离时传 -StorageDir/-ConfigDir 覆盖
    $envLine = '/c'
    if ($StorageDir) { $envLine += ' set "OVERSEER_STORAGE_DIR=' + $StorageDir + '" &&' }
    if ($ConfigDir) { $envLine += ' set "OVERSEER_CONFIG_DIR=' + $ConfigDir + '" &&' }
    if (-not $NoSidecar) {
        $envLine += ' set "OVERSEER_SIDECAR=' + "$script:OverseerRepo\sidecar\bin\Debug\net9.0-windows\overseer-uia-sidecar.exe" + '" &&'
    }
    $envLine += ' target\debug\terminal-overseer.exe > tauri-run.log 2>&1'
    Start-Process -FilePath "cmd.exe" -ArgumentList $envLine `
        -WorkingDirectory "$script:OverseerRepo\app\src-tauri" -WindowStyle Hidden
    Start-Sleep -Seconds 8
    Get-Process terminal-overseer -ErrorAction SilentlyContinue | Select-Object Id, MainWindowTitle
}

function Stop-OverseerDev {
    Get-Process terminal-overseer, overseer-uia-sidecar -ErrorAction SilentlyContinue | Stop-Process -Force
    Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
        Where-Object { $_.CommandLine -like '*vite*5173*' } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 1
    $left = Get-Process terminal-overseer, overseer-uia-sidecar -ErrorAction SilentlyContinue
    if ($left) { "still running: $($left.ProcessName -join ', ')" } else { "已全部关闭" }
}

function Get-OverseerWindowText {
    Add-Type -AssemblyName UIAutomationClient | Out-Null
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $cond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ClassNameProperty, "Tauri Window")
    $w = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $cond)
    if (-not $w) { return @("window not found") }
    $docCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document)
    $d = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $docCond)
    if (-not $d) { return @("document not ready") }
    $textCond = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)
    $texts = $d.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCond)
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    foreach ($t in $texts) { $t.Current.Name }
}

function Test-OverlayTransparency {
    # 采样远离ペット的屏幕点；全部是 #1E1E2E(30,30,46) → overlay 不透明；否则透明 ✓
    param([int[][]]$Points)
    Add-Type -AssemblyName System.Windows.Forms | Out-Null
    Add-Type -AssemblyName System.Drawing | Out-Null
    $size = [System.Windows.Forms.SystemInformation]::PrimaryMonitorSize
    $bmp = New-Object System.Drawing.Bitmap $size.Width, $size.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
    if (-not $Points) { $Points = @(@(700,500), @(1000,600), @(400,650), @(1500,400)) }
    $results = foreach ($p in $Points) {
        $c = $bmp.GetPixel($p[0], $p[1])
        [pscustomobject]@{
            X = $p[0]; Y = $p[1]; R = $c.R; G = $c.G; B = $c.B
            IsOpaqueDark = ($c.R -eq 30 -and $c.G -eq 30 -and $c.B -eq 46)
        }
    }
    $g.Dispose(); $bmp.Dispose()
    $opaqueCount = ($results | Where-Object IsOpaqueDark).Count
    [pscustomobject]@{
        Pixels  = $results
        Verdict = if ($opaqueCount -eq $results.Count) { "OPAQUE ✗ (#1E1E2E 全屏)" } else { "TRANSPARENT ✓" }
    }
}
