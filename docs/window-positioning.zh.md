# Window Positioning（窗口方位布局引擎）

## 概述

引擎为所有非 pet 窗口（ChatPanel、Component 卡片）计算相对于 pet 的唯一最优方位。核心原则：

- **最近**：离 pet 越近越好
- **不重叠**：新窗口不与 pet 及任何已有窗口重叠
- **最优**：满足前两条的前提下，方位尽可能接近期望方向

引擎接受一个 preferred 方向（16 方位之一，见下），返回新窗口中心的世界坐标。

---

## 坐标系与建模

### 符号

| 符号 | 含义 |
|---|---|
| `A` | pet 中心点（固定） |
| `B` | 新窗口中心（待求） |
| `m⃗` | preferred 方向的单位向量 |
| `θ` | `∠(m⃗, AB⃗)`，两向量夹角，取值 `[0, π]` |
| `|AB|` | pet 中心到新窗口中心的欧氏距离 |

### 价值函数

```
V(B) = α · θ² + β · |AB|    (α, β > 0)
```

- `α ≫ β`（方向优先级压倒性高于距离），具体值由实现微调
- debug 模式可通过 DevTools 或端点覆盖 `α`、`β`
- `gap`：窗口间最小间距，默认 12px，可配置
- 极小值点 = 全局唯一最优解

---

## 单调性：唯一极小值

将 B 约束在一条线段 `CD` 上（直线段，长度 `l`），参数 `t ∈ [0, l]` 表示 B 位置。设 A 不在直线 CD 上。

推导得（见附录）：

```
f'(t) = ψ'(t) · F(t)
F(t) = 2α(ψ(t) − c) + (β/d) · (t−a)·√((t−a)²+d²)
F'(t) = 2α·ψ'(t) + (β/d) · (2(t−a)²+d²)/√((t−a)²+d²)  > 0
```

**结论**：`F(t)` 严格单调递增 → `f'(t)` 至多有一零点 → `f(t)` 单谷 → 每条边线上有唯一极小值（内部驻点或端点）。

---

## 边线分解

引擎把 pet 周围可用空间分解为若干**边线段（CD 段）**，形成搜索域。

### 禁止区外扩

设新窗口尺寸 `W_new × H_new`。`gap` 为最小间距（默认 12px）。

每个已有窗口（含 pet）以其 BBox 为基，**各向四边外扩**：

```
outerX = gap + W_new / 2
outerY = gap + H_new / 2
```

外扩后的矩形区域是禁止区——新窗口**中心**不得落入。

### 并集外边界与 CD 段提取

1. 收集所有已有窗口（含 pet）的禁止区矩形
2. 所有禁止区矩形的并集 = 障碍物多边形
3. 提取障碍物的**外边界**
4. 外边界上每条**凸边段** = 一条合法 CD 段

新窗口中心落在 CD 段上时，窗口边缘与最近障碍物恰好贴 `gap`，不重叠。

---

## 三分搜索

每条 CD 段上 f(t) 单谷 → 三分法（ternary search）收敛到局部极小。

```
function ternarySearch(CD, A, mPref, α, β, tol):
    l, r = 0, |CD|
    while r - l > tol:
        m1 = l + (r-l)/3
        m2 = r - (r-l)/3
        if V(m1) < V(m2): r = m2
        else:             l = m1
    return V((l+r)/2)
```

全搜索域 = 所有 CD 段。每条 CD 段上局部极小 → 全局 min 取各段中的最小值。对应的 `B` 坐标即为唯一最优方位。

---

## 引擎接口

```typescript
interface PositioningEngine {
  /** 注册窗口（尺寸），返回最优位置；"auto" 按屏幕剩余空间最大方位现算 */
  place(window: WindowSpec, preferred: Direction | "auto"): Point;

  /** dismiss：占区与布局记忆一并忘记 */
  remove(windowId: string): void;

  /** 用户隐藏：释放占区但保留布局记忆（重开 place 原位恢复） */
  release(windowId: string): void;

  /** 拖拽结束回写真实位置为新跟随基准（manual 标记） */
  updateCenter(windowId: string, center: Point): Point | null;

  /** 恢复坐标：现算 petCenter + offset（无快照） */
  restorePositions(petCenter: Point): { id: string; center: Point }[];
}

interface WindowSpec {
  id: string;
  width: number;   // 窗口声明尺寸
  height: number;
  gap?: number;     // 自定义间距，默认 12（常量 DEFAULT_GAP，无全局可配字段）
}
```

### 藏/恢复

- pet 移动（拖拽开始）：整层系统藏——占区原地保留，无快照；
- pet 移动结束：现算 `pet_new + offset` 恢复（restorePositions）——offset 是 pet 相对偏移，天然随 pet 平移，无需记录-回放；
- 拖拽回写：松手经 `updateCenter` 把 OS 真实位置换算为新跟随基准（manual 标记），后续 pet 移动以拖后位置为基准跟随。

### Direction 枚举

16 方位英文名，0 = 顶（north），顺时针 22.5° 递增：

```
enum Direction {
  n, nne, ne, ene,     // 0-3
  e, ese, se, sse,     // 4-7
  s, ssw, sw, wsw,     // 8-11
  w, wnw, nw, nnw,     // 12-15
}
```

| 值 | 名称 | 角度 | 说明 |
|---|---|---|---|
| 0 | `n` | 0° | 正上方 |
| 4 | `e` | 90° | 正右侧 |
| 7 | `sse` | 157.5° | ChatPanel 默认 |
| 8 | `s` | 180° | 正下方 |
| 12 | `w` | 270° | 正左侧 |

### ChatPanel 默认

ChatPanel 的 preferred 方向固定 **`sse`**（docs/chat-panel.md §布局；0 = 顶，顺时针）。layout 略偏移以匹配视觉习惯。

### gap 配置

窗口间最小间距（px）：`WindowSpec.gap` 可逐窗口覆盖；全局默认 = `DEFAULT_GAP` 常量 12（positioning/engine.ts；当前无 Config 字段）。

---

## 算法流程

```
输入: petCenter, newWindow{size, preferred}, occupiedWindows[]

1. 计算禁止区：对 pet + occupiedWindows 各自 BBox 四向外扩
2. 取禁止区并集外边界 → CD 段列表
3. for each CD 段:
     ternarySearch(CD, petCenter, preferredAngle, α, β)
4. 取所有局部极小中的全局最小 → B 坐标
5. 返回 B
```

时间复杂度：O(k · log(1/ε))，其中 k = CD 段数量（≤ 2 × occupiedCount）。

---

## 附录：价值函数单调性推导

设线段 CD 在 x 轴，`C = (0,0)`，`D = (l,0)`。`B(t) = (t, 0)`。

`A = (a, d)`，`d > 0`（A 不在 CD 上）。

```
|AB| = d(t) = √((t−a)² + d²)
ψ(t) = atan2(−d, t−a)    // AB 方向角，单调递增 (ψ'(t) > 0)
θ(t) = |ψ(t) − c|         // 与 preferred 方向角 c 的夹角
f(t) = α·θ² + β·d(t)
```

```
f'(t) = 2αθ(t)θ'(t) + β·d'(t)
      = ψ'(t) [2α(ψ(t)−c) + β·d'(t)/ψ'(t)]
```

由 `d'(t) = (t−a)/d(t)`、`ψ'(t) = d/d(t)²` 得 `d'(t)/ψ'(t) = (t−a)·d(t)/d`。

令 `F(t) = 2α(ψ(t)−c) + (β/d)·(t−a)·d(t)`。

`F'(t) = 2α·ψ'(t) + (β/d)·(2(t−a)²+d²)/d(t)` > 0（各项正）。

→ `F(t)` 严格单调递增 → `f'(t)` 至多一零点 → `f(t)` 单谷 → 唯一极小值（内部驻点或端点）。
