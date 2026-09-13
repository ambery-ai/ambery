// 阅读位移引擎——实验 02 v=2 算法抽出来的模块（experiments/02-scroll-model/prototype/scroll-lab.html）。
//
// 抽出来是为了让"实验 / 产物"共用同一份代码：先前移植时我按描述重写了一遍，丢了判定时机与
// 曲线归属，手感全变，所以这里是逐条搬。原版里那几条**不生效**的判定当年也一并搬了进来
// （当时选择显式保留死代码，免得"顺手修好"混成行为差异），现已删除：
//   · 450ms 落地锁——`motionB.lockDir` 从未被置为非 0，`lockAt` 从未被赋值（差值是 NaN，比较恒假）；
//   · `lastWheelAt` 只写不读——它唯一的读点就在上面那条死判定里。
// 真正的保护是"glide 期间不接输入" + 落到边界后每帧按方向把它压回边界。
//
// 另注：原版的 `lastReset`（"同一落点不重复触发"的闩）**在原版里是活的**（每帧复位分支读它、写它），
// 但本模块从来没有实现那条判定，这个字段只被写、不影响任何决策，所以一并删掉。要不要把原版那条闩
// 补回来是另一件事：补回来会改变"glide 中途掉头后是否立刻重新吸附"的行为。
//
// 两条曲线各管一件事：
//   · 滚轮尾巴：指数收尾，时间常数 TAU（跟手、有惯性）
//   · 跨段吸附：定时 glide（三次缓出，RAMP_SECONDS），精确停在段边界

/** 滚轮尾巴的时间常数（秒） */
export const TAU = 0.7;
/** 跨段吸附的时长（秒） */
export const RAMP_SECONDS = 0.5;

/** 三次缓出 */
export const easeOutCubic = (p: number) => 1 - Math.pow(1 - p, 3);

export type TierSource = () => HTMLElement[];

export type MotionState = {
  /** 位置落在第几段（0 起） */
  section: number;
  /** 引擎认为的位置 */
  doc: number;
  /** 目标位 */
  target: number;
  /** 是否正在跨段 glide */
  gliding: boolean;
  /** glide 的原因（调试用） */
  snapInfo: { why: string } | null;
};

export type MotionOptions = {
  viewH?: () => number;
  maxDoc?: () => number;
  scrollY?: () => number;
  scrollTo?: (y: number) => void;
  now?: () => number;
  /** 每帧结束后回调（读数/高亮用） */
  onTick?: (state: MotionState) => void;
  /**
   * 额外的"唤醒"位移比例（0~1，默认 0 = 原版行为）：滚一格时立即把位置推进 delta × 该比例，
   * 用来消掉"第一像素要等下一帧"。原版没有这一手，需要时显式开。
   */
  wakeRatio?: number;
};

/**
 * 创建引擎。`tiers()` 返回各段元素；引擎不自己查 DOM（页面重挂内容后由调用方的新数组体现）。
 */
export function createMotion(tiers: TierSource, options: MotionOptions = {}) {
  const viewH = options.viewH ?? (() => window.innerHeight);
  const maxDoc = options.maxDoc ?? (() => Math.max(0, document.documentElement.scrollHeight - viewH()));
  const readY = options.scrollY ?? (() => window.scrollY);
  const writeY = options.scrollTo ?? ((y: number) => window.scrollTo(0, y));
  const clock = options.now ?? (() => performance.now());
  const wakeRatio = options.wakeRatio ?? 0;

  const motor = {
    doc: readY(),
    target: readY(),
    glide: null as { t0: number; from: number; to: number } | null,
    snapInfo: null as { why: string } | null,
  };
  let dir = 0;
  let section = 0;
  let raf = 0;
  let last = clock();

  /** 位置落在第几段 */
  const sectionIndexOf = (y: number, sections: HTMLElement[]) => {
    let i = 0;
    sections.forEach((s, n) => {
      if (y >= s.offsetTop - 1) i = n;
    });
    return i;
  };

  const snapshot = (): MotionState => ({
    section,
    doc: Math.round(motor.doc),
    target: Math.round(motor.target),
    gliding: !!motor.glide,
    snapInfo: motor.snapInfo,
  });

  /** 跨段 glide（原版 push 与"每帧复位"两条路径共用同一个字段） */
  const startGlide = (landing: number, now: number, why: string) => {
    motor.glide = { t0: now, from: readY(), to: landing };
    motor.target = landing;
    motor.snapInfo = { why };
  };

  /** 滚一格（原版 push：段号变了才吸附；参照系是"目标位"） */
  const push = (delta: number): boolean => {
    const sections = tiers();
    if (!sections.length) return false;
    const up = delta < 0;
    const from = motor.target;
    const to = Math.max(0, Math.min(maxDoc(), from + delta));
    const fromSeg = sectionIndexOf(from, sections);
    const toSeg = sectionIndexOf(to, sections);
    if (fromSeg === toSeg) {
      motor.target = to;
      // 可选唤醒：原版 wakeRatio = 0（不介入），需要消掉"等一帧"时由调用方显式开
      if (wakeRatio > 0) {
        const immediate = Math.max(0, Math.min(maxDoc(), readY() + delta * wakeRatio));
        motor.doc = immediate;
        writeY(Math.round(immediate));
      }
      return false;
    }
    const boundary = up ? sections[fromSeg].offsetTop : sections[toSeg].offsetTop;
    const landing = Math.max(0, Math.min(maxDoc(), up ? boundary - viewH() : boundary));
    startGlide(landing, clock(), "越过第 " + (up ? fromSeg + 1 : toSeg) + " 段边线");
    return true;
  };

  /** 每帧推进一步；返回当前状态 */
  const step = (now = clock()): MotionState => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    const sections = tiers();
    const k = 1 - Math.pow(0.001, dt / TAU); // 帧率无关的收尾系数

    if (motor.glide) {
      // 跨段 glide：定时曲线走完，落点是算出来的位置
      const p = Math.min(1, (now - motor.glide.t0) / (RAMP_SECONDS * 1000));
      motor.doc = motor.glide.from + (motor.glide.to - motor.glide.from) * easeOutCubic(p);
      if (p >= 1) {
        motor.doc = motor.glide.to;
        motor.target = motor.doc;
        motor.glide = null;
      }
    } else {
      // 外部滚动（触屏 / 键盘 / 拖动）不吞掉
      if (Math.abs(readY() - motor.doc) > 1.5) motor.doc = motor.target = readY();
      motor.doc += (motor.target - motor.doc) * k;
      if (Math.abs(motor.target - motor.doc) < 0.4) motor.doc = motor.target;
    }
    const want = Math.round(motor.doc);
    if (readY() !== want) writeY(want);

    // 每帧复位（原版注释：落点由方向唯一决定，两个方向各自收敛）
    if (!motor.glide) {
      const top = Math.round(readY());
      const sTop = sectionIndexOf(top, sections);
      const sBottom = sectionIndexOf(top + viewH() - 1, sections);
      if (sTop !== sBottom) {
        const down = dir >= 0;
        const landing = down
          ? Math.min(maxDoc(), sections[sBottom].offsetTop)
          : Math.max(0, sections[sTop].offsetTop + sections[sTop].offsetHeight - viewH());
        if (Math.abs(landing - top) > 2) {
          startGlide(
            landing,
            now,
            "跨段复位（" + (down ? "往下 → 压到第 " + (sBottom + 1) + " 段起点" : "往上 → 顶回第 " + (sTop + 1) + " 段末尾") + "）",
          );
        }
      }
    }

    section = Math.max(0, Math.min(sections.length - 1, sectionIndexOf(readY(), sections)));
    const state = snapshot();
    options.onTick?.(state);
    return state;
  };

  /** 滚轮输入（原版 wheel 处理：记方向 → glide 中掉头则取消 → push） */
  const input = (delta: number): { glided: boolean } => {
    const next = Math.sign(delta);
    if (!next) return { glided: false };
    const changed = next !== dir;
    dir = next;

    if (motor.glide) {
      if (changed) {
        // 掉头：取消 glide，位置交还读者
        motor.glide = null;
        motor.doc = motor.target = readY();
      }
      return { glided: true };
    }
    return { glided: push(delta * 1.15) };
  };

  const start = () => {
    const loop = (t: number) => {
      step(t);
      raf = requestAnimationFrame(loop);
    };
    last = clock();
    raf = requestAnimationFrame(loop);
  };

  return {
    push,
    input,
    step,
    start,
    state: snapshot,
    direction: () => dir,
    /** 小点跳段：同一套过渡曲线（原版 glideTo） */
    glideTo: (to: number) => {
      const target = Math.max(0, Math.min(maxDoc(), to));
      if (Math.abs(target - readY()) < 1) return;
      motor.glide = { t0: clock(), from: readY(), to: target };
      motor.target = target;
    },
    destroy: () => cancelAnimationFrame(raf),
  };
}
