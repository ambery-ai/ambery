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

环境变量（脚本自动设置，指向隔离临时目录，避免吃到真实配置）：
  OVERSEER_CONFIG_DIR / OVERSEER_STORAGE_DIR — 默认 %TEMP%/overseer-brain-{config,storage}
"""

import json
import os
import subprocess
import sys
import tempfile

NOTIFY_THRESHOLD = 80  # 旧 fixture：hook 内容 ≥ 80 字才通知（真实判断由 LLM 做）

EXE = os.path.join(os.path.dirname(__file__), "..", "core", "target", "debug", "overseer-debug.exe")


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


def tool_name_of(messages, tool_call_id):
    """由 tool_call_id 反查 tool 名（向前找发起它的 assistant tool_calls 消息）"""
    for m in reversed(messages):
        for c in m.get("tool_calls") or []:
            if c.get("id") == tool_call_id:
                return c.get("name")
    return None


def instance_overview(messages):
    """从 system prefix（i=0 消息）抠「## 当前实例状态」下的 - 行"""
    if not messages or not messages[0].get("content"):
        return "（无实例）"
    _, sep, after = messages[0]["content"].partition("## 当前实例状态")
    if not sep:
        return "（无实例）"
    lines = [l for l in after.splitlines() if l.startswith("- ")]
    return "\n".join(lines) if lines else "（无实例）"


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
    """旧 decide() 全规则复刻。返回响应行列表（空列表 = 沉默）。"""
    if not messages:
        return []
    tail = messages[-1]
    role = tail.get("role")
    content = tail.get("content") or ""

    if role == "tool":
        # fetch_terminal → 汇总回复；通知类动作已通过 Component 表达 → 沉默
        if tool_name_of(messages, tail.get("tool_call_id")) == "fetch_terminal":
            return [f"c [debug] 查到：{truncate(content, 120)}"]
        return []

    if role == "user":
        if "具体" in content or "怎么回事" in content:
            inst = last_noteworthy_instance(messages)
            if inst:
                return [f't fetch_terminal {json.dumps({"instance": inst}, ensure_ascii=False)}']
            return ["c [debug] 没有可查的实例记录"]
        return [f"c [debug] 收到：{content}（Queue 共 {len(messages)} 条）"]

    if role == "system":
        # 新实例注册（Example A）：问候 (・ω・)ノ + 展示实例一览
        if content.startswith("新实例 ") and content.endswith(" 已注册"):
            overview = instance_overview(messages)
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
