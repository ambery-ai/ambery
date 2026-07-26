/**
 * 拖拽 debounce：首次调用 onFirst，持续触发重置计时，停 delayMs 后调 onDone。
 * onDone 接收最后一次调用的 arg，保证使用最新状态。
 */
export function dragDebounce<T>(
  onFirst: () => void,
  onDone: (latest: T) => void,
  delayMs: number,
): (latest: T) => void {
  let started = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let last: T | undefined;

  return (latest: T) => {
    last = latest;
    if (!started) { started = true; onFirst(); }
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => { started = false; onDone(last!); last = undefined; }, delayMs);
  };
}
