# emthin — 嵌套 Wayland 合成器，以渲染文档呈现

> **把 Wayland 应用当作分页渲染文档中的一张张插图。**

emthin 是一个嵌套 Wayland 合成器，它的界面是一个
[mathed](https://github.com/voxell-tech/velyst) **文档**。Emacs 已经移除；
合成器中不再有任何 Elisp 布局引擎或窗口管理策略。屏幕上显示的是一份
Typst 渲染的文档，每个 Wayland 应用都**作为文档中的一张插图**出现 ——
就像 PDF 中的图片，大小和位置都由文本决定。

```
#1 Terminal demo #2 \app(#1, #2, 640, 400, "foot", launch: "foot")
#3 Chat #4 \app(#3, #4, 320, 240, "chat")
```

`\app` 语句在页面流中预留一个 `640 × 400` 的位置。当某个应用绑定到这张
插图时，它的实时界面就会被精确合成到该位置之上。修改这些数字就会改写
文档、重新排版，并重新配置应用 —— 文档是布局的权威，而不是合成器状态。

`launch:` 指定填充**空**位置时运行的命令。没有应用绑定的插图会画成一个带
标签的占位框；把光标移到它上面按 `Return`，或者直接点击它，就会运行
`launch:` 在该位置启动应用。它写在文档里而不是由合成器记住，理由与尺寸
相同：文档才是一个 `\app` 意味着什么的权威。省略它位置依然占位，只是没有
可启动的东西。

---

## 状态

Emacs 界面已被替换（设计与工作分解见 `docs/REWRITE_PLAN.md`）。合成器底座
—— smithay、DBus fcitx5 桥接、xwayland-satellite、剪贴板代理、layer-shell、
指针约束、ext-workspace-v1 —— 保持不变。

**尚未实现**（已在计划中，见重写计划）：

- 交互式应用启动器（`Ctrl+Shift+Return` 目前只是重启第一个休眠插图保存的命令）；
- 休眠插图的"点击启动"交互（Enter 重启绑定在启动器按键上，而非逐图提示）；
- 插图的自由二维放置 —— 按设计，插图位于文档流中。

## 构建

```sh
cargo build --release          # emthin 可执行文件
cargo clippy --workspace -- -D warnings
cargo test -p emthin
```

在 NixOS 上构建需要 `pkg-config`、`libxkbcommon` 和 `glib` 在搜索路径上
（`emthin-dbus` crate 链接 `gio`）。详见
[`docs/build-notes.md`](docs/build-notes.md)。

## 使用

```sh
# 以空文档启动
emthin

# 打开文档并把两个应用启动到插图中
emthin --doc notes.typ --spawn foot --spawn "firefox --new-window"

# 使用指定状态文件的嵌套会话
emthin --session-file /tmp/emthin-demo.loro
```

每个 `--spawn` 是一整条命令行，支持引号与反斜杠转义。除非你明确要求，
合成器不会自动启动任何东西：文档即会话，重启后其中的插图会回到**休眠**
状态（一个带标题的空框）。启动一个合成器从未见过的应用时，会在文档末尾
追加一条新的 `\app`，标题取自该应用的标题。

### 键位

| 按键 | 动作 |
|---|---|
| `Ctrl+Shift+Return` | 重启第一个休眠插图保存的命令 |
| `PgUp` / `PgDn` | 上一页 / 下一页 |
| `Home` / `End` | 文档开头 / 结尾 |
| `Ctrl+Home` / `Ctrl+End` | 行首 / 行尾 |
| `Ctrl+Shift+M` | 克隆当前插图（添加镜像） |
| `Escape` / `Ctrl+G` | 从插图把焦点交回文档 |
| `XF86WakeUp` | 在当前插图与文档之间切换焦点 |

编辑按键与普通编辑器一致：可打印键插入，`BackSpace` / `Delete` 删除一个
字素，方向键移动（`Shift` 选择，`Ctrl` 按词），`Ctrl+A` 全选，`Ctrl+Z` /
`Ctrl+Y` 撤销重做，`Ctrl+C` / `Ctrl+X` / `Ctrl+V` 使用宿主剪贴板。

### 插图

- **点击**插图即可聚焦其应用；**拖动边缘**（6px 以内）可调整大小 ——
  `\app` 参数会被改写，应用随之重新配置。尺寸按 8px 网格吸附。
- **镜像**是两条共享同一绑定 id 的 `\app` 语句，同一个应用会同时渲染在
  两处。`Ctrl+Shift+M` 会克隆一条插图语句。
- 其他页上的插图保持应用运行但进入空闲 —— 它们会停止接收 frame 回调，
  直到你切回所在页面。

## 控制协议

通过 Unix socket 的 JSON-RPC 2.0 协议，可让状态栏、脚本或未来的外部前端
观察并驱动文档。详见 [`docs/ipc.md`](docs/ipc.md)。

## 设计

- `docs/REWRITE_PLAN.md` —— 重写设计与工作分解。
- `AGENTS.md` —— 架构、模块地图，以及重写后仍然成立的合成器陷阱。

## 许可与致谢

emthin 采用 **GPL-3.0**，见 [`LICENSE`](LICENSE)。

- 文档引擎：[mathed](https://github.com/voxell-tech/velyst)
  （`mathed_core`、`mathed_mini`）—— MIT OR Apache-2.0，以依赖形式使用。
- 画布与 grab 机制改编自 [driftwm](https://github.com/) —— GPL-3.0。

MIT 与 Apache 许可的依赖可组合进 GPL-3.0 二进制文件，其自身许可条款不变。