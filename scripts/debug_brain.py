#!/usr/bin/env python3
"""debug_brain.py — overseer-debug 的外部 LLM 大脑（纯 mock 的决策源）。

复刻旧 Rust DebugAgent 的全部规则，证明 debug CLI 协议（@@@ 帧 + c/t/空行）
表达力完备：core 零逻辑，决策全在这个脚本里。

用法：
  python scripts/debug_brain.py            # 启动 overseer-debug 并接管其 stdin/stdout
另开终端打事件：
  curl -X POST http://127.0.0.1:47600/hook -H 'Content-Type: application/json' `
    -d '{"event":"session_start","instance":"ft","project":"p","content":"启动"}'

协议：
  core → brain: @@@QUEUE_BEGIN / 每行一条消息 JSON / @@@QUEUE_END / @@@PROMPT ...
  brain → core: `c <文本>` | `t <tool名> <json参数>`（可多行）| 空行提交
  非帧行（server 日志）原样透传到本脚本 stdout。

  帧 = 完整请求（docs/storage.md）：[现拼请求头] + Queue 全部消息 + [Autonomy 状态末端]。
  请求头无实例清单（diff 事件化）——roster 由本脚本从 Queue 事件流重建。

环境变量（脚本自动设置，指向隔离临时目录，避免吃到真实配置）：
  OVERSEER_CONFIG_DIR / OVERSEER_STORAGE_DIR — 默认 %TEMP%/overseer-brain-{config,storage}
"""

import json
import os
import subprocess
import sys
import tempfile

NOTIFY_THRESHOLD = 80  # 旧 fixture：hook 内容 ≥ 80 字才通知（真实判断由 LLM 做）

EXE = os.path.join(os.path.dirname(__file__), "..", "target", "debug", "overseer-debug.exe")


# ── 旧 Rust DebugAgent 规则的 Python 复刻 ──

def parse_hook_msg(content: str):
    """解析「{instance} 完成，Context 已更新（{len} 字）。…」及兜底扫描同构消息"""
    if "，Context 已更新（" not in content:
        return None
    head, rest = content.split("，Context 已更新（", 1)
    for suffix in (" 兜底扫描发现变化", " 完成"):
        if head.endswith(suffix):
            head = head[: -len(suffix)]
            break
    if " 字" not in rest:
        return None
    len_str = rest.split(" 字", 1)[0]
    try:
        return head, int(len_str)
    except ValueError:
        return None


def is_head(m) -> bool:
    """现拼请求头（帧首条）：含 kaomoji 表段，不落 Queue"""
    return m.get("role") == "system" and "## 颜文字映射" in (m.get("content") or "")


def is_autonomy(m) -> bool:
    """Autonomy 状态末端（帧末条）：[face: key, motion: key]，不落 Queue"""
    c = m.get("content") or ""
    return m.get("role") == "system" and c.startswith("[face: ") and c.endswith("]")


def strip_frame(messages):
    """帧 = [请求头] + Queue + [Autonomy 末端] → 纯 Queue 部分"""
    body = messages[1:] if messages and is_head(messages[0]) else list(messages)
    if body and is_autonomy(body[-1]):
        body = body[:-1]
    return body


def tool_name_of(messages, tool_call_id):
    """由 tool_call_id 反查 tool 名（向前找发起它的 assistant tool_calls 消息）"""
    for m in reversed(messages):
        for c in m.get("tool_calls") or []:
            if c.get("id") == tool_call_id:
                return c.get("name")
    return None


def instance_overview(messages):
    """从 Queue 事件流重建实例清单（diff 事件化，docs/harness.md 规则 3）：
    注册 = Processing；完成 = Idle；归零重 diff 的全景消息直接解析 - 行"""
    roster = {}
    for m in messages:
        if m.get("role") != "system":
            continue
        c = m.get("content") or ""
        if c.startswith("新实例 ") and c.endswith(" 已注册"):
            roster[c[len("新实例 "):-len(" 已注册")]] = "Processing"
        elif c.startswith("实例全景同步"):
            for line in c.splitlines()[1:]:
                # 「- {name} [{Status}] project={...}」
                if line.startswith("- ") and " [" in line:
                    name = line[2:].split(" [", 1)[0]
                    status = line.split(" [", 1)[1].split("]", 1)[0]
                    roster[name] = status
        elif "完成，Context 已更新" in c:
            parsed = parse_hook_msg(c)
            if parsed:
                roster[parsed[0]] = "Idle"
    if not roster:
        return "（无实例）"
    return "\n".join(f"- {name} [{status}]" for name, status in roster.items())


def last_noteworthy_instance(messages):
    """user 追问时定位 fetch_terminal 目标：优先最近一个触发通知的完成实例，无则最近完成"""
    latest = None
    for m in reversed(messages):
        if m.get("role") != "system" or not m.get("content"):
            continue
        parsed = parse_hook_msg(m["content"])
        if not parsed:
            continue
        inst, length = parsed
        if length >= NOTIFY_THRESHOLD:
            return inst
        if latest is None:
            latest = inst
    return latest


def truncate(s: str, n: int) -> str:
    return s[:n] + "…" if len(s) > n else s


def decide(messages):
    """旧 decide() 全规则复刻（适配新帧：先剥请求头/Autonomy 末端）。返回响应行列表（空 = 沉默）。"""
    body = strip_frame(messages)
    if not body:
        return []
    tail = body[-1]
    role = tail.get("role")
    content = tail.get("content") or ""

    if role == "tool":
        # fetch_terminal → 汇总回复；通知类动作已通过 Component 表达 → 沉默
        if tool_name_of(body, tail.get("tool_call_id")) == "fetch_terminal":
            return [f"c [debug] 查到：{truncate(content, 120)}"]
        return []

    if role == "user":
        if "具体" in content or "怎么回事" in content:
            inst = last_noteworthy_instance(body)
            if inst:
                return [f't fetch_terminal {json.dumps({"instance": inst}, ensure_ascii=False)}']
            return ["c [debug] 没有可查的实例记录"]
        return [f"c [debug] 收到：{content}（Queue 共 {len(body)} 条）"]

    if role == "system":
        # 新实例注册（Example A）：问候 (・ω・)ノ + 展示实例一览
        if content.startswith("新实例 ") and content.endswith(" 已注册"):
            overview = instance_overview(body)
            return [
                f't set_autonomy {json.dumps({"face": "(・ω・)ノ", "ttlMs": 3000}, ensure_ascii=False)}',
                "t call_component " + json.dumps({
                    "spec": {
                        "id": "roster",
                        "type": "text_card",
                        "title": "实例一览",
                        "text": overview,
                        "direction": "auto",
                    }
                }, ensure_ascii=False),
            ]
        parsed = parse_hook_msg(content)
        if parsed:
            inst, length = parsed
            if length >= NOTIFY_THRESHOLD:
                return [
                    "t set_autonomy " + json.dumps({
                        "face": "✧*｡٩(ˊᗜˋ*)و✧*｡", "motion": "bounce", "ttlMs": 5000,
                    }, ensure_ascii=False),
                    "t call_component " + json.dumps({
                        "spec": {
                            "id": f"notify-{inst}",
                            "type": "text_card",
                            "title": f"{inst} 完成",
                            "text": f"[debug] {inst} 干完了（{length} 字），去看看吧",
                            "direction": "auto",
                        }
                    }, ensure_ascii=False),
                ]
        return []  # 沉默（len < 阈值 / 其他 system 消息）

    return []


# ── 进程包装：spawn overseer-debug，接管 stdin/stdout ──

def main() -> int:
    # Windows GBK 控制台打不了颜文字：决策行含 kaomoji，stdout 强制 UTF-8
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if not os.path.exists(EXE):
        print(f"[brain] 找不到 {EXE}，先 cargo build --bin overseer-debug", flush=True)
        return 1

    env = dict(os.environ)
    tmp = tempfile.gettempdir()
    env.setdefault("OVERSEER_CONFIG_DIR", os.path.join(tmp, "overseer-brain-config"))
    env.setdefault("OVERSEER_STORAGE_DIR", os.path.join(tmp, "overseer-brain-storage"))

    print(f"[brain] 启动 {EXE}", flush=True)
    print(f"[brain] config={env['OVERSEER_CONFIG_DIR']} storage={env['OVERSEER_STORAGE_DIR']}", flush=True)
    proc = subprocess.Popen(
        [EXE],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,  # 行缓冲
        env=env,
    )

    def respond(lines):
        for line in lines:
            print(f"[brain] → {line}", flush=True)
            proc.stdin.write(line + "\n")
        proc.stdin.write("\n")  # 空行提交
        proc.stdin.flush()

    in_frame = False
    messages = []
    try:
        for line in proc.stdout:
            line = line.rstrip("\n")
            if line == "@@@QUEUE_BEGIN":
                in_frame, messages = True, []
            elif line == "@@@QUEUE_END":
                in_frame = False
                respond(decide(messages))
            elif in_frame:
                try:
                    messages.append(json.loads(line))
                except json.JSONDecodeError:
                    print(f"[brain] 帧内无法解析：{line}", flush=True)
            else:
                print(line, flush=True)  # server 日志透传（含 @@@PROMPT / @@@ERR）
    except KeyboardInterrupt:
        pass
    finally:
        proc.terminate()
    return 0


if __name__ == "__main__":
    sys.exit(main())
