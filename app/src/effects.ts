// effects — 前端非 readonly @tauri-apps/api 调用上报。
// fire-and-forget：不 await、错误吞掉（上报失败不破坏窗口逻辑）；
// 高频 kind 按 key 250ms 去抖打包成一条（payload 附 count）。

type Payload = Record<string, unknown>;

/** 打包 kind（其余即时上报）：window_moved/window_resized 跟随拖动高频、event_emit 协调事件高频 */
const PACKED_KINDS = new Set(["window_moved", "window_resized", "event_emit"]);
const PACK_DEBOUNCE_MS = 250;

interface PackedEntry {
  kind: string;
  payload: Payload;
  count: number;
  timer: ReturnType<typeof setTimeout>;
}
const packed = new Map<string, PackedEntry>();

/** 上报一条前端动作（effect.jsonl origin=frontend）。
 *  Tauri 模式走 record_effect command；headless/browser 调试走 RemoteBridge 的 POST /effect。
 *  两端点都是 core 的 record_frontend_effect 单点。 */
export function reportEffect(kind: string, payload: Payload = {}): void {
  if (PACKED_KINDS.has(kind)) {
    const key = `${kind}:${String(payload.window ?? payload.event ?? "")}`;
    const existing = packed.get(key);
    if (existing) {
      existing.payload = { ...payload };
      existing.count += 1;
      clearTimeout(existing.timer);
      existing.timer = setTimeout(() => flush(key), PACK_DEBOUNCE_MS);
    } else {
      packed.set(key, {
        kind,
        payload: { ...payload },
        count: 1,
        timer: setTimeout(() => flush(key), PACK_DEBOUNCE_MS),
      });
    }
    return;
  }
  void send(kind, payload);
}

function flush(key: string): void {
  const entry = packed.get(key);
  if (!entry) return;
  packed.delete(key);
  void send(entry.kind, { ...entry.payload, count: entry.count });
}

function remoteBase(): string {
  const port = (globalThis as Record<string, unknown>).__AMBERY_PORT__ ?? "47600";
  return `http://127.0.0.1:${port}`;
}

async function send(kind: string, payload: Payload): Promise<void> {
  try {
    if ("__TAURI_INTERNALS__" in window) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("record_effect", { kind, payload });
    } else {
      await fetch(`${remoteBase()}/effect`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ kind, payload }),
      });
    }
  } catch {
    // fire-and-forget：忽略上报失败
  }
}
