import "./styles.css";
import { Autonomy } from "./autonomy";
import { BrowserMockBridge, createBridge } from "./bridge";
import { View } from "./view";

async function main() {
  const bridge = createBridge();
  const mount = document.getElementById("app")!;
  const view = new View(mount);
  const autonomy = new Autonomy(bridge, (e) => view.setExpression(e));
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
  }
  window.__overseer = debug as Window["__overseer"];
}

main();
