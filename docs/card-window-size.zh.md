# Card 窗口尺寸

[English](card-window-size.md) | 中文

> - **本文档范围**——本文定义 Card 窗口尺寸如何确定：唯一来源、向 Card 文件的投影、触发重算的输入变化，以及从投影到建窗的路径。坐标单位与显示缩放换算归 [window-follow.zh.md](window-follow.zh.md)；Card 文件字段清单归 [components.zh.md](components.zh.md)；pet 窗口尺寸归 [pet-window-size.zh.md](pet-window-size.zh.md)。

## 定义

一个 Card 窗口渲染恰好一张 Card（[multi-window.zh.md](multi-window.zh.md)）。它的尺寸是该 OS 窗口的物理尺寸，不是 Card 内容盒的尺寸：Card 的 `min-width` / `max-width` 是窗口内部的样式上限，窗口才是用户看到的那层包裹。

## 唯一来源

窗口尺寸由一个尺寸计算产出，输入是 Card 内容、生效的字体标识，以及该 Card 渲染的布局常量（字号、行高、内边距、chrome）。该计算由 `@chenglou/pretext` 承担：`prepareWithSegments()` 对内容块做一次性的分段与 canvas 测量、按（字体, 分段）缓存宽度，`measureNaturalWidth()` 给出该块的本征宽度（不折行，硬换行仍分段），`layout()` 行走分段 advance、在给定宽度与行高下推出行数，且 `height = lineCount × lineHeight`；块高叠加 chrome 常量即窗口尺寸。

该计算不读 Card 已渲染布局的任何几何——不读 Card 及其后代的 `getBoundingClientRect`、`offsetWidth`、`offsetHeight`、`scrollHeight`——因此结果不取决于当前是否有窗口在显示这张 Card，也不取决于窗口多大。唯一的 DOM 访问是 pretext 的 emoji 宽度修正：文本可能含 emoji 时，它把 canvas 测出的 emoji 宽度与一个隐藏 inline-block span 的宽度比较，每个字体算一次并缓存差值。它从不读 Card。

渲染消费尺寸，绝不产出尺寸。产者只有一个，所以 Card 窗口只有一个尺寸定义，而不是一个算出来的加一个量出来的。

## 输入来源

| 输入 | 家 |
|---|---|
| Card 内容 | Card 文件的 `component`（[docs/components.zh.md](components.zh.md) §Card 文件（.card.json）） |
| 字体标识 | Config 的字体维度（`ui.font`）——用户所选字体的解析后字体族 |
| 布局常量 | 渲染契约：Card 的字号、字重、字距、行高、内边距、chrome 与宽度上下限，由 Card 样式与组件构建代码声明 |

`layoutVersion` 是后两者的标识——字体标识 + 布局常量。任一变化即让全部投影失效：每张存活 Card 重算，并重写其 `layoutVersion`。

## 字体标识

Card 字体是用户选择，所以尺寸计算不假定任何字体栈。由此两条约束：

- **字体必须命名**。泛型族（`sans-serif`、`system-ui`、`-apple-system`、`ui-sans-serif`）按平台与系统版本解析到不同字面，用它算出的尺寸连同一台机器上都不可复现；它可以用来渲染 Card，但不作尺寸权威。
- **测量前字体必须已加载**。canvas 测量对未加载的字体会静默替换成回退字面，算出的尺寸与渲染不一致。因此计算会等到字体可用；加载不成的字体经错误通道呈现（[docs/errors.zh.md](errors.zh.md)），而不是按回退字体量一个数。

| 字体类别 | 可作尺寸权威 | 跨机器可复现 |
|---|---|---|
| app 打包字体 | 是 | 是 |
| 平台已有的命名字体 | 是 | 否——同一张 Card 在另一平台可算出不同尺寸 |
| 远程字体、泛型族、或加载失败的用户自带字体 | 否 | 否 |

量出的度量属于解析后的那个字面，不属于字体列表：列表在测量前解析，第一个存在的族胜出。列表里排第一的族缺失时，度量会静默来自下一个族，所以同一份配置在两台机器上可以算出两个尺寸（[docs/concrete-insight.zh.md](concrete-insight.zh.md) §Card 尺寸推导）。

跨机器可复现是**性质**，不是每张卡片的警告：字体未随 app 打包的 Card 在另一台机器上可以算出不同尺寸，这是预期行为。选字体的设置界面提示一次；单张卡片不告警。

## 内容块

Card 内容按块测量，因为 Card body 是一串布局规则各异的 markdown 块：

| 块 | 高度贡献 |
|---|---|
| 标题、段落、列表项、引用块 | 行数 × 行高 + 块间距 |
| 围栏代码块 | 行数 × 代码行高；该块横向滚动而不折行 |
| 表格 | 每行取各列折行后最高的单元格；列宽取该列单元格的 max-content 宽度，再钳到 Card 宽度上限 |
| 分隔线 | 一个固定高度 |

行内内容（强调、加粗、行内代码、链接）改变某行的宽度，不改变块模型。Card 的 chrome——header、内边距与边框——在块总高之上加一次。

Card 正文由少量封闭的块种类拼成。每种块只有一种布局与一条高度规则，各自独立可算；块内不混布局种类。每个块的高度都由其内容与布局常量推导出来，绝不来自计算无法预知的元素尺寸。块内渲染可以用 flex、grid 或绝对定位，前提是该块的规则覆盖其几何。尺寸来自媒体的块声明自己的尺寸——宽 + 高，或宽 + 宽高比——于是计算无需等待加载即可推导。

Card body 的高度上限是一个固定常量——与宽度上限同一个布局常量——超出部分在 Card 内滚动。该上限不依赖显示器：依赖显示器的上限会让投影变成显示器相关，本契约不允许。

宽度有上下限，不是自由推导的：Card 的宽度取其各块与 header 中最宽者，再钳到宽度下限与上限之间。内容比下限窄仍取下限宽；比上限宽则在上限处折行。header 标题是**单行不折行**并以省略号裁切，所以再长的标题也不会把 Card 撑过宽度上限，且恒贡献一行高。没有最小高度：Card 的高度就是 chrome 加各块的高度。

## 投影

算出的尺寸投影进 Card 文件，供其它组件直接读而不必重跑计算：`_meta.layout.size` 存 `[width, height]`，单位 CSS px；`_meta.layout.layoutVersion` 存决定尺寸的输入标识（布局常量 + 字体标识）——值不等于当前输入标识的投影即过期。字段清单与文件契约归 [components.zh.md](components.zh.md) §Card 文件（.card.json）；本文只拥有尺寸字段的语义。

Card 文件的唯一写入方是 core；前端产出尺寸，经 Card 写入通路请求落盘。

`size` 是派生状态而非用户选择：与 `_meta.layout.offset`、`_meta.layout.manual` 不同，没有任何用户动作直接编辑它，用户拖拽只改变位置。

## 单位与显示缩放

投影里的尺寸以 CSS px 存储，与显示缩放无关：缩放系数不是尺寸计算的输入，也不出现在 `layoutVersion` 里；卡片被挪到缩放不同的显示器上时，投影不变。

转成物理像素只在进入 engine 与 OS 的边界发生一次（[window-follow.zh.md](window-follow.zh.md) §坐标单位契约）。物理尺寸按**目标显示器**的缩放系数推导——即窗口被放置的那块屏，而不是 Card 窗口自己启动时的 `devicePixelRatio`——并**向上取整**，因此分数缩放不会给出比内容更小的窗口。放置与尺寸一起计算，并在窗口落到目标屏之后一次应用，物理尺寸从不按窗口起始所在的那块屏推导。

## 重算触发

| 输入变化 | 是否重算 |
|---|---|
| agent 同 id 更新 Card 内容 | 是 |
| 用户新增 todobox 条目 | 是 |
| 用户勾选或取消 todobox 条目 | 否——`done` 变了，但行数与宽度都没变 |
| Card chrome 文案（UI 语言切换） | 是——按钮文字参与宽度 |
| 字体标识或布局常量 | 是，全部 Card |
| 主题切换（颜色、圆角） | 否——主题是纯视觉变更 |

重算只有单一入口：上述每个触发都汇到同一个函数，且算出的尺寸与产出它的内容在同一次 Card 文件写入里落盘，因此投影不会出现「内容新、尺寸旧」的配对。启动时每张存活 Card 的投影都重算一遍；`layoutVersion` 已与当前输入一致的投影不重写。

## 建窗

Card 窗口按 Card 文件投影里的尺寸创建，尺寸在建窗之前读取。Card 窗口从不自己改尺寸：不调 `setSize`，也不存在 show 之后的测量循环。create / reuse / close 的决策仍归壳（[case-runner.zh.md](case-runner.zh.md) §窗口决策上提）；本文只拥有建窗时使用的那个尺寸。

## 与 pet 尺寸的关系

pet 尺寸是对脸宽、缩放与动作的闭式公式（[pet-window-size.zh.md](pet-window-size.zh.md)）：pet 的内容是固定的，公式就是全部契约。Card 的内容是任意的，所以尺寸由内容算出。两者都是唯一来源，都不从活动窗口量取。

## 验收

尺寸推导属于渲染侧逻辑，因此 case-runner 不测它：该 harness 观测的是前端逻辑——store、动作层与窗口接线（`docs/case-runner.zh.md` §观测边界）。本契约改以真实渲染为准验收：把一张 Card 的推导尺寸与同一张 Card 实际渲染后占据的尺寸逐类型对比，覆盖会给块模型施压的各种内容形态。对比在卡片实际渲染的引擎里跑——折行的容差与若干断行行为按引擎校准——所以容差取该引擎自己的，而不是所有平台共用一个数。
