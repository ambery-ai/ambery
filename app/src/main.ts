import "./styles.css";
import { Autonomy } from "./autonomy";
import { BrowserMockBridge, createBridge } from "./bridge";
import { ComponentManager } from "./components";
import { View } from "./view";

async function main() {
  const bridge = createBridge();
  const mount = document.getElementById("app")!;
  const view = new View(mount);
  const autonomy = new Autonomy(bridge, (e) => view.setExpression(e));
  // Component 以 View 中心为锚点（concepts §5）
  new ComponentManager(mount, bridge, () => view.center());
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
  }
  window.__overseer = debug as Window["__overseer"];
}

main();
