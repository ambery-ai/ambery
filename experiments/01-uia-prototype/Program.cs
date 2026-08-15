using System.Diagnostics;
using System.Text.RegularExpressions;
using System.Windows.Automation;

namespace AmberyUiaPrototype;

class Program
{
    // ════════════════════════════════════════════════════
    //  配置
    // ════════════════════════════════════════════════════
    const int POLL_INTERVAL_MS = 5_000;         // 轮询间隔
    const int STABLE_THRESHOLD = 3;             // N 次无变化 → 判定完成
    const int MIN_TEXT_LENGTH = 50;             // 忽略太短的文本

    // ════════════════════════════════════════════════════
    //  状态
    // ════════════════════════════════════════════════════
    static readonly Dictionary<string, TabState> Tabs = new();
    static int _pollCount;

    static async Task Main(string[] args)
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        Console.WriteLine($"👀 Ambery 启动");
        Console.WriteLine($"   轮询: {POLL_INTERVAL_MS / 1000}s × {STABLE_THRESHOLD} 次稳定 → 判定完成");
        Console.WriteLine("   按 Ctrl+C 退出\n");

        while (true)
        {
            await PollAllTabs();
            await Task.Delay(POLL_INTERVAL_MS);
        }
    }

    // ════════════════════════════════════════════════════
    //  主轮询
    // ════════════════════════════════════════════════════

    static async Task PollAllTabs()
    {
        try
        {
            var hwnd = FindWtHwnd();
            if (hwnd == IntPtr.Zero) return;

            var root = AutomationElement.FromHandle(hwnd);
            var tabs = FindTabs(root);
            if (tabs.Count == 0) return;

            _pollCount++;
            if (_pollCount % 6 == 1)
                Console.WriteLine($"💓 #{_pollCount} 监控 {tabs.Count} 个标签页…");

            foreach (var (tabEl, name) in tabs)
            {
                try
                {
                    await SwitchToTab(tabEl);
                    var text = ReadTerminalText(root);
                    if (text == null || text.Length < MIN_TEXT_LENGTH) continue;

                    var hash = (long)text.Length << 32 | (uint)text.GetHashCode(StringComparison.Ordinal);

                    if (Tabs.TryGetValue(name, out var prev))
                    {
                        if (prev.LastHash == hash)
                        {
                            prev.StableCount++;
                            if (prev.StableCount == STABLE_THRESHOLD && prev.NotifiedHash != hash)
                            {
                                prev.NotifiedHash = hash;
                                OnDone(name, text, prev);
                            }
                        }
                        else
                        {
                            prev.LastHash = hash;
                            prev.LastText = text;
                            prev.StableCount = 0;
                        }
                    }
                    else
                    {
                        Tabs[name] = new TabState { Name = name, LastHash = hash, LastText = text };
                    }
                }
                catch { /* skip bad tab */ }
            }
        }
        catch { /* WT minimized */ }
    }

    // ════════════════════════════════════════════════════
    //  完成回调 ← 在这里接你的干活 Agent
    // ════════════════════════════════════════════════════

    static void OnDone(string tabName, string fullText, TabState state)
    {
        var summary = Tail(fullText, 500);
        Console.ForegroundColor = ConsoleColor.Yellow;
        Console.WriteLine($"\n🔔 {DateTime.Now:HH:mm:ss} 「{tabName}」完成");
        Console.ResetColor();
        Console.WriteLine($"   摘要: {Tail(fullText, 300)}");
        Console.WriteLine($"   总计 {fullText.Length} 字符\n");

        // 🔌 这里接干活 Agent
        // e.g. Process.Start("claude", $"\"看看这个: {summary}\"");
    }

    // ════════════════════════════════════════════════════
    //  UIA wrappers
    // ════════════════════════════════════════════════════

    static IntPtr FindWtHwnd()
    {
        var procs = Process.GetProcessesByName("WindowsTerminal");
        return procs.FirstOrDefault(p => p.MainWindowHandle != IntPtr.Zero)
                   ?.MainWindowHandle ?? IntPtr.Zero;
    }

    static List<(AutomationElement El, string Name)> FindTabs(AutomationElement root)
    {
        var list = new List<(AutomationElement, string)>();
        var tabs = root.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.TabItem));

        foreach (AutomationElement t in tabs)
        {
            var name = t.Current.Name?.Trim();
            if (!string.IsNullOrWhiteSpace(name))
                list.Add((t, name));
        }
        return list;
    }

    static async Task SwitchToTab(AutomationElement tabEl)
    {
        if (tabEl.TryGetCurrentPattern(SelectionItemPattern.Pattern, out object? p)
            && p is SelectionItemPattern sel)
        {
            sel.Select();
            await Task.Delay(200);
        }
    }

    static string? ReadTerminalText(AutomationElement root)
    {
        var els = root.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.IsTextPatternAvailableProperty, true));

        foreach (AutomationElement el in els)
        {
            if (el.Current.ClassName == "TermControl"
                && el.TryGetCurrentPattern(TextPattern.Pattern, out object? p)
                && p is TextPattern tp)
            {
                return tp.DocumentRange.GetText(-1);
            }
        }
        return null;
    }

    // ════════════════════════════════════════════════════
    //  工具
    // ════════════════════════════════════════════════════

    static string Tail(string text, int chars)
    {
        var start = Math.Max(0, text.Length - chars);
        var s = Regex.Replace(text[start..], @"^\s*\r?\n+", "");
        return s.Length > 200 ? s[..200] + "…" : s;
    }
}

class TabState
{
    public string Name { get; set; } = "";
    public string LastText { get; set; } = "";
    public long LastHash { get; set; }
    public long NotifiedHash { get; set; }
    public int StableCount { get; set; }
}
