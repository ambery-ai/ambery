// 原型桩的最小公共件：单独成文件只为断环——stubs.ts 要用 cards.ts 的样例，
// cards.ts 要用 noop，若 noop 定义在 stubs.ts 就成了循环导入（TDZ 报错）。
export const noop = () => {};
