// Kitchen Sink 入口（契约见 docs/kitchen-sink.md）。
// 隔离：应用与样张是两个入口，样张产物写自己的输出目录、不进应用 dist。
// 没有 createWindowShell：本页不需要 IPC/store/窗口适配器——这正是「点什么都不会发生」的根。
import { mount } from "svelte";
import "../styles/index.css";
import "./page.css";
import KitchenSink from "./KitchenSink.svelte";

export function main() {
  if (!("__TAURI_INTERNALS__" in window)) {
    // 浏览器整页：不透明底（Tauri 窗口是透明的，两者底色语义不同）
    document.documentElement.classList.add("browser");
  }
  document.title = "ambery · kitchen sink";
  const target = document.getElementById("app");
  if (target) mount(KitchenSink, { target });
}

// 独立入口自启动：本页不经应用的路由表（那才是隔离的前提），入口自己点亮自己。
void main();
