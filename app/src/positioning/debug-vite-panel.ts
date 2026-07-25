// positioning/debug-vite-panel — Vite dev 调试面板（prod build tree-shaking 剔除）
// 注：全模块仅 import.meta.env.DEV 时有效，prod 时 import 消去

import type { PositioningEngine } from "./engine";
import { computeCDSegments } from "./geometry";
import { Direction } from "./types";

export class DebugPositioningPanel {
  static readonly DEV_ONLY = !import.meta.env.PROD;
  private el: HTMLDivElement;
  private engine: PositioningEngine;
  private petCenter = { x: 0, y: 0 };
  private petSize = { w: 72, h: 40 };

  constructor(engine: PositioningEngine) {
    this.engine = engine;
    this.el = document.createElement("div");
    this.el.id = "debug-positioning-panel";
    this.el.style.cssText =
      "position:fixed;bottom:8px;right:8px;background:rgba(30,30,46,0.95);color:#cdd6f4;padding:12px;border-radius:10px;font-size:12px;z-index:99999;max-width:240px";
    document.body.appendChild(this.el);
    this.render();
  }

  setPet(center: { x: number; y: number }, size: { w: number; h: number }) {
    this.petCenter = center;
    this.petSize = size;
  }

  private syncPetPos() {
    const v = document.getElementById("view");
    if (v) {
      const vr = v.getBoundingClientRect();
      this.petCenter = { x: vr.x + vr.width / 2, y: vr.y + vr.height / 2 };
      this.petSize = { w: Math.round(vr.width), h: Math.round(vr.height) };
      this.engine.registerPet(this.petCenter, this.petSize);
    }
  }

  private render() {
    const a = this.engine.alpha;
    const b = this.engine.beta;
    this.el.innerHTML = `
      <div style="font-weight:600;margin-bottom:8px">🔧 Positioning Debug</div>
      <label>α <input id="dbg-alpha" type="range" min="0" max="100" step="0.1" value="${a}" style="width:100px"> <span>${a}</span></label><br>
      <label>β <input id="dbg-beta" type="range" min="0" max="100" step="0.1" value="${b}" style="width:100px"> <span>${b}</span></label>
      <div style="margin-top:6px">
        <button id="dbg-place">Place</button>
        <button id="dbg-clear">Clear</button>
        <button id="dbg-obstacles">Obs 1s</button>
        <input id="dbg-replay" placeholder="n,n,n,wnw,wnw…" style="width:120px;margin-top:4px">
        <button id="dbg-replay-btn">Replay</button>
        <select id="dbg-dir">${Object.values(Direction)
          .filter((v) => typeof v === "string")
          .map((n) => `<option>${n}</option>`)
          .join("")}</select>
      </div>
      <div id="dbg-log" style="margin-top:6px;max-height:120px;overflow-y:auto;font-family:monospace"></div>
    `;

    this.el.querySelector<HTMLInputElement>("#dbg-alpha")!.oninput = (ev) => {
      this.engine.alpha = Number((ev.target as HTMLInputElement).value);
      this.render();
    };
    this.el.querySelector<HTMLInputElement>("#dbg-beta")!.oninput = (ev) => {
      this.engine.beta = Number((ev.target as HTMLInputElement).value);
      this.render();
    };
    this.el.querySelector("#dbg-place")!.addEventListener("click", () => {
      this.syncPetPos();
      const sel = this.el.querySelector<HTMLSelectElement>("#dbg-dir")!.value;
      const dir = Direction[sel as keyof typeof Direction] as Direction;
      const pos = this.engine.place(
        { id: `debug-${Date.now()}`, width: 150, height: 100 },
        dir,
        this.petCenter,
        this.petSize,
      );
      // 渲染红框指示新窗口位置
      const mark = document.createElement("div");
      mark.className = "dbg-place-mark";
      mark.style.cssText = `position:fixed;left:${pos.x - 75}px;top:${pos.y - 50}px;width:150px;height:100px;border:2px dashed red;pointer-events:none;z-index:9998`;
      document.body.appendChild(mark);
      this.log(`place ${sel} → (${Math.round(pos.x)}, ${Math.round(pos.y)})`);
    });
    this.el.querySelector("#dbg-clear")!.addEventListener("click", () => {
      document.querySelectorAll(".dbg-place-mark").forEach((el) => el.remove());
      // 清 engine 占区
      this.engine.clear();
      this.log("cleared all marks");
    });
    this.el.querySelector("#dbg-replay-btn")!.addEventListener("click", () => {
      this.syncPetPos();
      const raw = this.el.querySelector<HTMLInputElement>("#dbg-replay")!.value;
      const dirs = raw.split(",").map((s) => s.trim()).filter(Boolean);
      const results: string[] = [];
      for (const d of dirs) {
        const dir = Direction[d as keyof typeof Direction] as Direction;
        if (dir === undefined) { results.push(`${d}: unknown`); continue; }
        const pos = this.engine.place(
          { id: `replay-${Date.now()}-${d}`, width: 150, height: 100 },
          dir, this.petCenter, this.petSize,
        );
        const mark = document.createElement("div");
        mark.className = "dbg-place-mark";
        mark.style.cssText = `position:fixed;left:${pos.x - 75}px;top:${pos.y - 50}px;width:150px;height:100px;border:2px dashed red;pointer-events:none;z-index:9998`;
        document.body.appendChild(mark);
        results.push(`${d}→(${Math.round(pos.x)},${Math.round(pos.y)})`);
      }
      this.log(`replay [${dirs.join(",")}]: ${results.join(" | ")}`);
    });
    this.el.querySelector("#dbg-obstacles")!.addEventListener("click", () => {
      this.syncPetPos();
      const occupied = [...document.querySelectorAll(".dbg-place-mark")].map((el) => {
        const r = (el as HTMLElement).getBoundingClientRect();
        return { center: { x: r.x + r.width / 2, y: r.y + r.height / 2 }, w: r.width, h: r.height };
      });
      const segs = computeCDSegments(
        this.petCenter, this.petSize,
        { id: "viz", width: 150, height: 100 }, occupied, 12,
      );
      const overlays: HTMLElement[] = [];
      for (const [C, D] of segs) {
        const line = document.createElement("div");
        const w = Math.max(Math.abs(D.x - C.x), 4);
        const h = Math.max(Math.abs(D.y - C.y), 4);
        line.style.cssText = `position:fixed;left:${Math.min(C.x, D.x)}px;top:${Math.min(C.y, D.y)}px;width:${w}px;height:${h}px;background:rgba(0,255,0,0.3);border:1px solid lime;pointer-events:none;z-index:9997`;
        document.body.appendChild(line);
        overlays.push(line);
      }
      setTimeout(() => overlays.forEach((el) => el.remove()), 1000);
      this.log(`shown ${segs.length} CD segments`);
    });
  }

  private log(msg: string) {
    const logEl = this.el.querySelector("#dbg-log")!;
    logEl.innerHTML += `<div>${msg}</div>`;
    logEl.scrollTop = logEl.scrollHeight;
  }
}
