import "./styles.css";

const hash = window.location.hash;

if (hash === "#cards") {
  import("./windows/cards").then((m) => m.main());
} else if (hash === "#chat") {
  import("./windows/chat-window").then((m) => m.main());
} else if (hash === "#menu") {
  import("./windows/menu").then((m) => m.main());
} else {
  import("./windows/pet").then((m) => m.main());
}
