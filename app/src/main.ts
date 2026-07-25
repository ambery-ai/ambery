import "./styles.css";

// 根据 hash 路由到不同窗口入口（docs/multi-window.md）
const hash = window.location.hash;

if (hash === "#cards") {
  import("./windows/cards").then((m) => m.main());
} else if (hash === "#chat") {
  import("./windows/chat-window").then((m) => m.main());
} else {
  import("./windows/pet").then((m) => m.main());
}
