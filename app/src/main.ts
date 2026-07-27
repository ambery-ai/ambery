import "./styles.css";

// 路由依据 = Tauri 窗口 label（一等公民，不经过 URL）；
// 浏览器模式（无 label）回退 hash。#menu hash 在 conf url 里会丢（可见性
// 初始化为 false 的窗口疑似踩 Tauri url fragment 怪癖），label 不会丢。
async function route() {
  let key = "";
  if ("__TAURI_INTERNALS__" in window) {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    key = getCurrentWindow().label;
  }
  if (!key) key = window.location.hash.replace("#", "");

  if (key.startsWith("card-")) {
    import("./windows/card-window").then((m) => m.main());
  } else if (key === "chat") {
    import("./windows/chat-window").then((m) => m.main());
  } else if (key === "menu") {
    import("./windows/menu").then((m) => m.main());
  } else {
    import("./windows/pet").then((m) => m.main());
  }
}

void route();
