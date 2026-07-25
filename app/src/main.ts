import "./styles.css";
import { Autonomy } from "./autonomy";
import { BrowserMockBridge, createBridge } from "./bridge";
import { ChatPanel } from "./chat";
import { ComponentManager } from "./components";
import { View, type Edge } from "./view";

async function main() {
  // Tauri 壳模式：背景透明（docs/tauri-shell.md），只显示ペット与卡片。
  // 必须加在 html 根元素——CSS 背景写在 html,body 两个选择器上，
  // class 挂在 body 而选择器是 html.tauri 时不匹配（已踩坑×2）
  if ("__TAURI_INTERNALS__" in window) document.documentElement.classList.add("tauri");
  const bridge = await createBridge();
  const mount = document.getElementById("app")!;
  const view = new View(mount);
  const autonomy = new Autonomy(bridge, (e) => view.setExpression(e));
  // RemoteBridge：Overseer 推送的 set_autonomy / config 变更直达 Autonomy
  bridge.onSetAutonomy?.((args) => autonomy.setAutonomy(args));
  bridge.onConfigChanged?.((cfg) => autonomy.updateConfig(cfg));
  // Component 以 View 中心为锚点（concepts §5）
  new ComponentManager(mount, bridge, () => view.center());
  // Chat Panel：View 右键吸附唤出 / 解除吸附关闭（concepts §3+§3a）
  const chatPanel = new ChatPanel(mount, bridge, () => view.center());
  let dockedEdge: Edge = "top";
  view.el.addEventListener("view:docked", (ev) => {
    dockedEdge = (ev as CustomEvent<{ edge: Edge }>).detail.edge;
    chatPanel.show(dockedEdge);
  });
  view.el.addEventListener("view:undocked", () => chatPanel.hide());
  // 面板被 × 关闭后，吸附态左键单击 View 重新唤出（docs/chat-panel.md）
  view.el.addEventListener("click", () => {
    if (view.isDocked() && !chatPanel.isVisible()) chatPanel.show(dockedEdge);
  });
  await autonomy.init();

  // Chrome DevTools 调试驱动接口：__overseer.setInstanceStatus(...) 等
  const debug: Partial<Window["__overseer"]> = {
    setAutonomy: (args) => autonomy.setAutonomy(args),
    viewState: () => ({
      docked: view.isDocked(),
      center: view.center(),
      face: document.getElementById("face")?.textContent ?? null,
      motion: view.el.dataset.motion ?? "still",
    }),
  };
  if (bridge instanceof BrowserMockBridge) {
    debug.setInstanceStatus = (n, s) => bridge.debugSetInstanceStatus(n, s);
    debug.addInstance = (n, s) => bridge.debugAddInstance(n, s);
    debug.notify = (n) => bridge.debugNotify(n);
    debug.clearNotifications = () => bridge.debugClearNotifications();
    debug.callComponent = (spec) => bridge.debugCallComponent(spec);
    debug.eventBuffer = () => bridge.debugEventBuffer();
    debug.flushEventBuffer = () => bridge.debugFlushEventBuffer();
    debug.appendMessage = (role, content) =>
      bridge.debugAppendMessage(role, content);
  }
  window.__overseer = debug as Window["__overseer"];
}

main();
