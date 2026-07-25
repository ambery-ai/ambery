// positioning/debug-vite-panel — Vite 开发调试面板：α/β 滑块 + 窗口注册面板

import type { PositioningEngine } from "./engine";
import { Direction } from "./types";

export class DebugPositioningPanel {
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

  private render() {
    const a = this.engine.alpha;
    const b = this.engine.beta;
    this.el.innerHTML = `
      <div style="font-weight:600;margin-bottom:8px">🔧 Positioning Debug</div>
      <label>α <input id="dbg-alpha" type="range" min="0" max="1" step="0.05" value="${a}" style="width:100px"> <span>${a}</span></label><br>
      <label>β <input id="dbg-beta" type="range" min="0" max="1" step="0.05" value="${b}" style="width:100px"> <span>${b}</span></label>
      <div style="margin-top:6px">
        <button id="dbg-place">Place Test Window</button>
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
      const sel = this.el.querySelector<HTMLSelectElement>("#dbg-dir")!.value;
      const dir = Direction[sel as keyof typeof Direction] as Direction;
      const pos = this.engine.place(
        { id: `debug-${Date.now()}`, width: 150, height: 100 },
        dir,
        this.petCenter,
        this.petSize,
      );
      this.log(`place ${sel} → (${Math.round(pos.x)}, ${Math.round(pos.y)})`);
    });
  }

  private log(msg: string) {
    const logEl = this.el.querySelector("#dbg-log")!;
    logEl.innerHTML += `<div>${msg}</div>`;
    logEl.scrollTop = logEl.scrollHeight;
  }
}
