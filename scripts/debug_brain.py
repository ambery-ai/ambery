#!/usr/bin/env python3
"""debug_brain.py — HTTP LLM 替换示例（docs/debug-agent.md §scripts/debug_brain.py）

本地 OpenAI 兼容 /chat/completions HTTP 服务器，「LLM 替换」的最小示例：
决策源逻辑内置在本脚本里，case-runner 以 debug 模式 `--brain-addr` 连它当 LLM 用
（docs/case-runner.md §用例）。

用法：
  python scripts/debug_brain.py --port 47777   # --port 可选，默认 47777

  # 另开终端（debug 模式 case 回放）：
  ambery-case <case> --brain-addr http://127.0.0.1:47777
  # 或 serve 宿主：
  ambery-case serve --brain-addr http://127.0.0.1:47777

内置最小阈值决策源：请求最后一条消息匹配「{name} 完成，Context 已更新（N 字）」
（hook_stop_content，i18n hook.stop.updated）且 N ≥ 80 → 回通知 tool（call_component
文本卡）；否则沉默（空 content 无 tool_calls，concepts §9b）。

wire 形态：core 的 OpenAiClient 只讲 SSE 流式（docs/streaming.md）——stream:true
请求以 SSE 应答（单 chunk + usage 帧 + [DONE]）；非流式请求（如 Compression 摘要
调用）以普通 JSON 应答。仅标准库，无第三方依赖。
"""

import argparse
import json
import re
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

NOTIFY_THRESHOLD = 80  # 最小阈值决策源：hook 内容 ≥ 80 字才通知（真实判断由 LLM 做）

# i18n hook.stop.updated（zh）："{name} 完成，Context 已更新（{len} 字）。评估是否通知。"
STOP_UPDATED = re.compile(r"^(.*?) 完成，Context 已更新（(\d+) 字）")


def decide(messages):
    """最小阈值决策源。返回 (content, tool_calls)；沉默 = ("", [])。

    请求 = [现拼请求头] + Context 全部消息 + [Autonomy 状态末端]（docs/storage.md），
    末尾几帧不一定是触发消息——自尾向前找最近一条 stop auto_read 消息判定。
    """
    for m in reversed(messages or []):
        if m.get("role") != "system":
            continue
        hit = STOP_UPDATED.match(m.get("content") or "")
        if not hit:
            continue
        if int(hit.group(2)) < NOTIFY_THRESHOLD:
            return "", []
        inst = hit.group(1)
        break
    else:
        return "", []
    # card id 语法（docs/components.md）：仅 A-Z a-z 0-9 _ - . /——实例名的间隔号等须清洗
    safe_id = re.sub(r"[^A-Za-z0-9_\-./]", "_", inst)
    spec = {
        "id": f"notify-{safe_id}",
        "type": "text_card",
        "title": f"{inst} 完成",
        "text": f"[brain] {inst} 完成了（{hit.group(2)} 字），去看看吧",
        "direction": "auto",
    }
    return "", [
        {
            "index": 0,
            "id": "brain-1",
            "type": "function",
            "function": {
                "name": "call_component",
                "arguments": json.dumps({"spec": spec}, ensure_ascii=False),
            },
        }
    ]


def chunk(cid, delta, finish=None, usage=None):
    """OpenAI chat.completion.chunk 帧"""
    v = {
        "id": cid,
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": "brain",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    }
    if usage:
        v["usage"] = usage
    return v


class BrainHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # 静默默认访问日志，决策打印走 stdout
        pass

    def do_POST(self):
        if self.path.rstrip("/") != "/chat/completions":
            self.send_error(404, "only /chat/completions")
            return
        try:
            length = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(length) or b"{}")
        except Exception as e:
            self.send_error(400, f"bad json: {e}")
            return

        content, tool_calls = decide(body.get("messages") or [])
        # 粗粒度 usage 真值（#16：usage 帧供压缩基准；按字符数估算）
        prompt_chars = sum(len(m.get("content") or "") for m in body.get("messages") or [])
        usage = {
            "prompt_tokens": max(1, prompt_chars // 2),
            "completion_tokens": max(1, len(content) // 2 + len(tool_calls) * 20),
        }
        cid = "chatcmpl-brain"

        if body.get("stream"):
            frames = []
            if tool_calls:
                frames.append(chunk(cid, {"role": "assistant", "tool_calls": tool_calls}))
                frames.append(chunk(cid, {}, finish="tool_calls", usage=usage))
            else:
                # 沉默或纯文本：单 chunk 全量给出（content 空 = 沉默）
                delta = {"role": "assistant"}
                if content:
                    delta["content"] = content
                frames.append(chunk(cid, delta, finish="stop", usage=usage))
            payload = "".join(
                f"data: {json.dumps(f, ensure_ascii=False)}\n\n" for f in frames
            ) + "data: [DONE]\n\n"
            raw = payload.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
        else:
            message = {"role": "assistant", "content": content or None}
            if tool_calls:
                message["tool_calls"] = [
                    {"id": tc["id"], "type": "function", "function": tc["function"]}
                    for tc in tool_calls
                ]
            resp = {
                "id": cid,
                "object": "chat.completion",
                "created": int(time.time()),
                "model": "brain",
                "choices": [
                    {"index": 0, "message": message, "finish_reason": "tool_calls" if tool_calls else "stop"}
                ],
                "usage": usage,
            }
            raw = json.dumps(resp, ensure_ascii=False).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        if tool_calls:
            print(f"[brain] → {tool_calls[0]['function']['name']}", flush=True)
        else:
            print("[brain] → 沉默", flush=True)


def main() -> int:
    # Windows GBK 控制台防乱码：stdout 强制 UTF-8
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description="debug_brain — OpenAI 兼容 HTTP LLM 替换示例")
    ap.add_argument("--port", type=int, default=47777)
    args = ap.parse_args()
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), BrainHandler)
    print(f"[brain] listening on http://127.0.0.1:{args.port}/chat/completions", flush=True)
    print(f"[brain] 阈值决策源：stop auto_read 内容 ≥ {NOTIFY_THRESHOLD} 字 → 通知，否则沉默", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
