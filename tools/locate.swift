// 定位 Ambery 所有窗口的位置/尺寸/可见性（macOS 版，对应 tools/locate.ps1）
//
// 用法：
//   swift tools/locate.swift                表格式输出窗口列表
//   swift tools/locate.swift --highlight    红框=在屏(可见) 绿框=不在屏(隐藏)，10 秒后消失
//
// 数据源：CGWindowListCopyWindowInfo（无需辅助功能/屏幕录制权限；
// 坐标系为 Quartz 全局坐标，单位=逻辑点；Y 从屏幕左上角向下增长）。

import CoreGraphics
import Foundation
import AppKit

let args = CommandLine.arguments
let highlight = args.contains("--highlight") || args.contains("-highlight")

// ── 枚举 ambery 窗口 ──
struct Win {
    var id: Int
    var title: String
    var layer: Int
    var onScreen: Bool
    var alpha: Float
    var x: Double
    var y: Double
    var w: Double
    var h: Double
}

guard let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] else {
    print("could not query window list")
    exit(1)
}

var wins: [Win] = []
for w in list {
    let owner = w[kCGWindowOwnerName as String] as? String ?? ""
    if owner != "ambery" { continue }
    let bounds = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    wins.append(Win(
        id: w[kCGWindowNumber as String] as? Int ?? -1,
        title: w[kCGWindowName as String] as? String ?? "",
        layer: w[kCGWindowLayer as String] as? Int ?? -1,
        onScreen: w[kCGWindowIsOnscreen as String] as? Bool ?? false,
        alpha: w[kCGWindowAlpha as String] as? Float ?? 1.0,
        x: bounds["X"] as? Double ?? 0,
        y: bounds["Y"] as? Double ?? 0,
        w: bounds["Width"] as? Double ?? 0,
        h: bounds["Height"] as? Double ?? 0
    ))
}

if wins.isEmpty {
    print("ambery not running")
    exit(1)
}

// 可见窗口层(>=5)在前，其余按 Y/X 排序
wins.sort { a, b in
    if a.layer >= 5 && b.layer >= 5 { return a.y == b.y ? a.x < b.x : a.y < b.y }
    return a.layer > b.layer
}

// ── 表格输出 ──
let hdr = String(format: "%-8@ %-6@ %-8@ %-5@ %6@ %6@ %6@ %6@",
                 "ID", "Layer", "OnScr", "Alpha", "X", "Y", "W", "H")
print(hdr)
for w in wins {
    print(String(format: "%-8d %-6d %-8@ %-5.2f %6.0f %6.0f %6.0f %6.0f  %@",
                 w.id, w.layer,
                 w.onScreen ? "yes" : "no",
                 w.alpha, w.x, w.y, w.w, w.h,
                 w.title as NSString))
}
print("Total: \(wins.count) window(s)")

// 屏幕录制权限未授权时，kCGWindowName / kCGWindowIsOnscreen 被置空/置 false
if wins.allSatisfy({ $0.title.isEmpty }) && wins.allSatisfy({ !$0.onScreen }) {
    print("提示：屏幕录制权限未授权——窗口标题与在屏状态不可用（系统设置→隐私与安全性→屏幕录制）；")
    print("      可见性请参考 Layer：5=常规窗口层（pet/chat/menu/shelf/card），0=辅助/隐藏层。")
}

// ── Highlight：红框(在屏)/绿框(不在屏) 边框标注 10 秒 ──
if highlight {
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)

    var panels: [NSWindow] = []
    // CG 坐标原点在左上；NSWindow 屏幕坐标原点在左下 → Y 翻转
    let screenH = CGDisplayBounds(CGMainDisplayID()).height
    for w in wins {
        let panel = NSWindow(
            contentRect: NSRect(x: w.x, y: screenH - w.y - w.h, width: w.w, height: w.h),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        panel.level = .screenSaver
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.ignoresMouseEvents = true
        panel.isReleasedWhenClosed = false

        let color: NSColor = w.onScreen ? .red : .green
        let box = NSBox(frame: NSRect(x: 0, y: 0, width: w.w, height: w.h))
        box.boxType = .custom
        box.borderWidth = 2
        box.borderColor = color
        box.fillColor = .clear
        panel.contentView = box
        panel.orderFrontRegardless()
        panels.append(panel)
    }
    print("红框=在屏 绿框=不在屏，10 秒后消失…")
    DispatchQueue.main.asyncAfter(deadline: .now() + 10) {
        for p in panels { p.close() }
        exit(0)
    }
    app.run()
}
