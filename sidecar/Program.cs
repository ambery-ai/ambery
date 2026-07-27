// UIA sidecar（docs/sidecar.md）：stdio JSON Lines 协议，UIA 逻辑移植自 exp01。
// stdin 每行一个 JSON 请求 → stdout 每行一个 JSON 响应。sidecar 不过滤文本（Filter 在 Rust 侧）。

using System.Text.Json;
using System.Text.Json.Nodes;
using System.Windows.Automation;
using System.Runtime.InteropServices;

namespace OverseerSidecar;

class Program
{
    static readonly JsonSerializerOptions JsonOpts = new() { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

    static int Main()
    {
        Console.InputEncoding = System.Text.Encoding.UTF8;
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        string? line;
        while ((line = Console.ReadLine()) != null)
        {
            JsonObject resp;
            try
            {
                var req = JsonNode.Parse(line)!.AsObject();
                resp = Dispatch(req);
            }
            catch (Exception ex)
            {
                resp = Err(ex.Message);
            }
            Console.Out.Flush();
            Console.WriteLine(resp.ToJsonString(JsonOpts));
            Console.Out.Flush();
        }
        return 0;
    }

    static JsonObject Dispatch(JsonObject req)
    {
        var cmd = req["cmd"]?.GetValue<string>() ?? "";
        switch (cmd)
        {
            case "list_windows":
            {
                // EnumWindows + DWM cloaked：全 VD 视野（cloaked 窗口只有窗口级信息，docs/sidecar.md §视野模型）
                var wins = new JsonArray();
                foreach (var (hwnd, title, cloaked) in Uia.ListWindowsAllVds())
                    wins.Add(new JsonObject { ["hwnd"] = hwnd.ToInt64(), ["title"] = title, ["cloaked"] = cloaked });
                return Ok(new JsonObject { ["windows"] = wins });
            }
            case "count_processes":
            {
                var name = req["name"]?.GetValue<string>() ?? "";
                var n = name.Length == 0 ? 0 : System.Diagnostics.Process.GetProcessesByName(name).Length;
                return Ok(new JsonObject { ["count"] = n });
            }
            case "list_tabs":
            {
                var hwnd = new IntPtr(req["hwnd"]!.GetValue<long>());
                var tabs = new JsonArray();
                var list = Uia.ListTabs(hwnd);
                for (int i = 0; i < list.Count; i++)
                    tabs.Add(new JsonObject { ["index"] = i, ["name"] = list[i].Name, ["selected"] = list[i].Selected });
                return Ok(new JsonObject { ["tabs"] = tabs });
            }
            case "find_tab":
            {
                var name = req["name"]!.GetValue<string>();
                var found = Uia.FindTab(name);
                if (found == null) return Err($"tab not found: {name}");
                var (hwnd, index, tabName) = found.Value;
                return Ok(new JsonObject { ["hwnd"] = hwnd.ToInt64(), ["index"] = index, ["name"] = tabName });
            }
            case "read_tab":
            {
                var hwnd = new IntPtr(req["hwnd"]!.GetValue<long>());
                var index = req["index"]!.GetValue<int>();
                var text = Uia.ReadTab(hwnd, index);
                return text == null ? Err("read failed") : Ok(new JsonObject { ["text"] = text });
            }
            case "read_active_tab":
            {
                var hwnd = new IntPtr(req["hwnd"]!.GetValue<long>());
                var text = Uia.ReadActiveTab(hwnd);
                return text == null ? Err("read failed") : Ok(new JsonObject { ["text"] = text });
            }
            default:
                return Err($"unknown cmd: {cmd}");
        }
    }

    static JsonObject Ok(JsonObject data)
    {
        data["ok"] = true;
        return data;
    }

    static JsonObject Err(string msg) => new() { ["ok"] = false, ["error"] = msg };
}

// UIA 封装（exp01 验证过的路径：EnumWindows → TabItem → Select → TermControl TextPattern）
static class Uia
{
    public static List<(IntPtr Hwnd, string Title)> ListWindows()
    {
        var result = new List<(IntPtr, string)>();
        var root = AutomationElement.RootElement;
        var wins = root.FindAll(TreeScope.Children,
            new PropertyCondition(AutomationElement.ClassNameProperty, "CASCADIA_HOSTING_WINDOW_CLASS"));
        foreach (AutomationElement w in wins)
        {
            var hwnd = new IntPtr(w.Current.NativeWindowHandle);
            if (hwnd != IntPtr.Zero)
                result.Add((hwnd, w.Current.Name ?? ""));
        }
        return result;
    }

    // ── 全 VD 枚举（EnumWindows + DWM cloaked；UIA root 只看得见当前 VD） ──
    private const int DWMWA_CLOAKED = 14;

    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(IntPtr hwnd, int attr, out int value, int size);

    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr hwnd, System.Text.StringBuilder cls, int max);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hwnd, System.Text.StringBuilder text, int max);

    /// 全 VD 的 CASCADIA 窗口：hwnd + 窗口标题（= 活动 tab 标题）+ cloaked 标记
    public static List<(IntPtr Hwnd, string Title, bool Cloaked)> ListWindowsAllVds()
    {
        var result = new List<(IntPtr, string, bool)>();
        EnumWindows((hwnd, _) =>
        {
            var cls = new System.Text.StringBuilder(256);
            GetClassName(hwnd, cls, 256);
            if (cls.ToString() != "CASCADIA_HOSTING_WINDOW_CLASS") return true;
            var title = new System.Text.StringBuilder(512);
            GetWindowText(hwnd, title, 512);
            _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, out var c, sizeof(int));
            result.Add((hwnd, title.ToString(), c != 0));
            return true;
        }, IntPtr.Zero);
        return result;
    }

    public static List<(string Name, bool Selected)> ListTabs(IntPtr hwnd)
    {
        var root = AutomationElement.FromHandle(hwnd);
        var result = new List<(string, bool)>();
        var tabs = root.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.TabItem));
        foreach (AutomationElement t in tabs)
        {
            var name = t.Current.Name?.Trim();
            if (string.IsNullOrWhiteSpace(name)) continue;
            var selected = t.TryGetCurrentPattern(SelectionItemPattern.Pattern, out object? p)
                && p is SelectionItemPattern sel && sel.Current.IsSelected;
            result.Add((name, selected));
        }
        return result;
    }

    public static (IntPtr Hwnd, int Index, string Name)? FindTab(string namePart)
    {
        foreach (var (hwnd, _) in ListWindows())
        {
            var tabs = ListTabs(hwnd);
            for (int i = 0; i < tabs.Count; i++)
            {
                if (tabs[i].Name.Contains(namePart, StringComparison.Ordinal))
                    return (hwnd, i, tabs[i].Name);
            }
        }
        return null;
    }

    public static string? ReadTab(IntPtr hwnd, int index)
    {
        var root = AutomationElement.FromHandle(hwnd);
        var tabs = root.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.TabItem));
        var real = new List<AutomationElement>();
        foreach (AutomationElement t in tabs)
        {
            if (!string.IsNullOrWhiteSpace(t.Current.Name))
                real.Add(t);
        }
        if (index < 0 || index >= real.Count) return null;
        var tab = real[index];
        // 已选中的 tab 无需切换（200ms 成本 + 不打扰用户）
        var alreadySelected = tab.TryGetCurrentPattern(SelectionItemPattern.Pattern, out object? p0)
            && p0 is SelectionItemPattern sel0 && sel0.Current.IsSelected;
        if (!alreadySelected)
        {
            if (tab.TryGetCurrentPattern(SelectionItemPattern.Pattern, out object? p)
                && p is SelectionItemPattern sel)
            {
                sel.Select();
                Thread.Sleep(200); // exp01 实测切换成本
            }
        }
        return ReadTermControl(root);
    }

    public static string? ReadActiveTab(IntPtr hwnd)
    {
        var root = AutomationElement.FromHandle(hwnd);
        return ReadTermControl(root);
    }

    static string? ReadTermControl(AutomationElement root)
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
}

