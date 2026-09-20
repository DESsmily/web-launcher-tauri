import { listen } from "@tauri-apps/api/event";

import { nextRestoreStep } from "./focus-policy";
import { FOCUSED_EVENT, RESTORED_EVENT, embedShouldBeVisible } from "./session";
import { claimMainWebviewFocus } from "./webview-focus";

/**
 * 主界面输入框的「焦点保住」。
 *
 * **现象**：把光标放进某个输入框（主界面的、或**标签页里内嵌页面的**），切屏（Alt+Tab、
 * 切虚拟桌面、最小化到托盘）再回来，输入框就丢了焦点 —— 看起来还得重新点一下才能接着打字。
 *
 * **原因**：一个窗口里同时存在多个 WebView2 实例，各被 wry 包在一个 `WRY_WEBVIEW` 容器里。
 * wry 把「宿主窗口拿到焦点」转成「WebView2 拿到焦点」的办法，是给**顶层窗口**挂一条子类过程
 * （`WM_SETFOCUS` → `MoveFocus`），而那条子类**只给非 child 的 webview 装**
 * （`if !is_child`，wry-0.55.1 仍未修，见 `tauri-apps/wry#1754`）——
 * 内嵌页面是 `add_child` 出来的 child，压根没装。于是切屏回来时没人把焦点转进该拿它的
 * 那个 webview，主界面和内嵌页面都可能中招。
 *
 * 症状看起来是「DOM 毫无变化」（`document.activeElement` 还指着那个输入框），因为丢的是
 * **系统级**键盘焦点 —— 那种情况下 `element.focus()` 是空操作。完整分析见 Rust 侧
 * `winfocus.rs`（Rust 负责系统级焦点并判断该归哪个 webview；这里只管主界面的 DOM）。
 *
 * **分工**：系统级焦点归 Rust（`winfocus`）；这里只管 DOM —— 记住用户之前在编辑哪个元素、
 * 光标在哪，窗口回来后把它补上。两边都必须**校验 + 重试**：抢焦点牵扯到系统、
 * WebView2 浏览器进程、渲染进程三方异步，单发一次不可靠。
 *
 * **两个坑（都踩过）**：
 * - **清记忆不能用 `focusout`**：切屏 / 最小化也会失焦，而那正是要恢复的场景。
 * - **干脆不要主动清记忆**：早先按「点击到别处」和「`focusin` 落到非输入元素」清过，
 *   两种都会被**程序化**的焦点变化误伤 —— 切屏回来时 WebView2/wry 会把焦点丢给页面
 *   第一个可聚焦元素（主界面就是左上角那个图标按钮），于是记忆被清、回来无目标可恢复，
 *   焦点就钉在那个图标上（日志里失败轮次一律是 `last=null`）。
 *   元素失效由 `isVisible` 挡掉就够了；恢复只认「最后编辑过的输入元素」。
 */

/** 可编辑（能接受文本输入 / 有光标）的元素 */
function asEditable(target: EventTarget | null): HTMLElement | null {
  if (!(target instanceof HTMLElement)) return null;
  if (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  ) {
    return target;
  }
  return target.isContentEditable ? target : null;
}

function isVisible(el: HTMLElement): boolean {
  // 视图切换后元素会被卸载（v-if），此时不该再往它身上补焦点
  return el.isConnected && el.getClientRects().length > 0;
}

let lastEditable: HTMLElement | null = null;
let lastCaret: { start: number; end: number } | null = null;

/** 记下当前编辑位置；range 类 input（date / number / checkbox …）读选区会抛，直接跳过 */
function rememberCaret(el: HTMLElement): void {
  if (!(el instanceof HTMLInputElement) && !(el instanceof HTMLTextAreaElement)) {
    return;
  }
  try {
    const start = el.selectionStart;
    if (start === null) return;
    lastCaret = { start, end: el.selectionEnd ?? start };
  } catch {
    /* 该 input 类型不支持选区 */
  }
}

function rememberEditable(el: HTMLElement | null): void {
  lastEditable = el;
  if (!el) lastCaret = null;
  else rememberCaret(el);
}

/** 把焦点（以及光标位置）补回元素 */
function refocus(el: HTMLElement): void {
  try {
    el.focus({ preventScroll: true });
  } catch {
    el.focus();
  }

  if (!lastCaret) return;
  if (!(el instanceof HTMLInputElement) && !(el instanceof HTMLTextAreaElement)) return;
  const max = el.value.length;
  const start = Math.min(lastCaret.start, max);
  const end = Math.min(lastCaret.end, max);
  try {
    el.setSelectionRange(start, end);
  } catch {
    /* 同上：不支持选区的类型忽略 */
  }
}

/**
 * 补焦点的重试点（毫秒）。
 *
 * 为什么要拉这么长：切屏回来后**不止我们一家**在动 DOM 焦点 —— WebView2/wry 那侧
 * 也会调 `controller.MoveFocus(PROGRAMMATIC)`（wry 挂在顶层窗口的 `WM_SETFOCUS` 子类里），
 * 而它是**异步**生效的、比我们晚。只试到 200ms 会被它盖掉，于是焦点停在
 * 页面第一个可聚焦元素上（主界面就是左上角那个图标按钮）。
 */
const RESTORE_DELAYS_MS = [0, 60, 200, 400, 800, 1500];

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

/**
 * 把焦点补回元素，带重试。
 *
 * 每轮先按 `nextRestoreStep` 判定该做什么 —— 关键是**只在 DOM 焦点真的丢了时**才
 * `focus()`：元素已经是 `document.activeElement` 时再 focus 是空操作，那种情况下丢的是
 * 系统级焦点，得等 Rust 那边要回来（见 `winfocus.rs`），这里只是等。
 */
async function restoreElementFocus(el: HTMLElement): Promise<void> {
  for (const delay of RESTORE_DELAYS_MS) {
    if (delay) await sleep(delay);
    const step = nextRestoreStep({
      tracked: lastEditable === el,
      visible: isVisible(el),
      activeIsTarget: document.activeElement === el,
      documentFocused: document.hasFocus(),
    });
    if (step === "abort" || step === "done") return;
    if (step === "focus") refocus(el);
    // "wait"：什么都不做，等下一轮再看
  }
}

/** 一次「窗口重新获得焦点」的完整处理 */
async function handleWindowFocused(): Promise<void> {
  // 内嵌页面正显示时，键盘焦点本来就该属于那个页面，不去抢
  if (embedShouldBeVisible()) return;

  // 先把键盘焦点要回主界面：即便没有正在编辑的元素也要做，
  // 否则焦点可能停在已经隐藏的内嵌页面上，主界面的键盘完全没反应
  await claimMainWebviewFocus();

  const el = lastEditable;
  if (!el) return;
  await restoreElementFocus(el);
}

let timer = 0;

/** 合并同一轮里的多个触发（tauri://focus、window focus、window://focused 会几乎同时到） */
function scheduleRestore(delay = 0): void {
  if (timer) window.clearTimeout(timer);
  timer = window.setTimeout(() => {
    timer = 0;
    void handleWindowFocused();
  }, delay);
}

/**
 * 「刚回到窗口」的守备窗口（毫秒）。
 *
 * 这期间做两件事：
 * 1. **事件驱动**：任何「不是用户上次编辑的那个字段」的聚焦都当成被程序化抢走
 *    （WebView2/wry 的「聚焦第一个可聚焦元素」），在**同一个事件循环里**立刻抢回来 ——
 *    这样连一帧都不会画出来，用户看不到闪烁（`focusin` 处理里见）；
 * 2. **轮询兜底**：每 40ms 再校验一次（幂等）。
 *
 * 用户一有动作（点击 / 按键）立刻停手 —— 此后焦点归他管，我们不抢。
 */
const WATCHDOG_MS = 1500;
const WATCHDOG_INTERVAL_MS = 40;
let watchdog = 0;
let watchdogUntil = 0;
/**
 * 窗口失焦期间置位。
 *
 * 回来后、**用户还没动作之前**，任何「不是上次编辑的那个字段」的聚焦都当成被
 * 程序化抢走。用「失焦时置位」而不是「获得焦点时置位」，是因为那个抢焦点的
 * `focusin` 有可能比 `window.focus` 更早派发 —— 靠获得焦点来开窗口就漏了。
 */
let pendingRestore = false;

/** 把焦点拉回「用户上次编辑的字段」（幂等；没有目标 / 内嵌页可见时什么都不做） */
function restoreNow(): void {
  const el = lastEditable;
  if (!el || !isVisible(el) || embedShouldBeVisible()) return;
  if (document.activeElement !== el) refocus(el);
}

function startWatchdog(): void {
  watchdogUntil = Date.now() + WATCHDOG_MS;
  if (watchdog) return;
  watchdog = window.setInterval(() => {
    if (Date.now() > watchdogUntil) {
      window.clearInterval(watchdog);
      watchdog = 0;
      return;
    }
    restoreNow();
  }, WATCHDOG_INTERVAL_MS);
}

function stopWatchdog(): void {
  watchdogUntil = 0;
  pendingRestore = false;
}

/**
 * 安装焦点恢复。触发时机都是「窗口刚从别的应用 / 托盘 / 最小化状态回来」：
 * - `window://focused`（Rust 的 `WindowEvent::Focused(true)` 转发，最可靠）；
 * - `tauri://focus`（框架自带的窗口焦点事件，做冗余）；
 * - `window focus`（WebView2 自己重新拿到键盘焦点时）。
 */
export async function installFocusRestore(): Promise<void> {
  // 记住「用户最后编辑过的输入元素」。
  //
  // **刻意不主动清空**：早先按「点击到别处」「焦点落到非输入元素」清过，两种都会误伤 ——
  // 切屏回来时 WebView2/wry 会**程序化地**把焦点丢给页面第一个可聚焦元素
  // （主界面就是工具栏左上角那个图标按钮），那次 `focusin`、以及随手点一下页面/工具栏，
  // 都会把记忆清掉，于是窗口回来时无目标可恢复、焦点就钉在那个图标上
  // （踩过；日志里失败轮次一律表现为 `last=null`）。
  // 元素失效由 `isVisible`（`isConnected` + 有布局盒）挡掉，不需要靠清记忆来兜。
  document.addEventListener(
    "focusin",
    (event) => {
      // 「刚回到窗口」期间：任何不是「用户上次编辑的那个字段」的聚焦，
      // 都当成被**程序化**抢走（WebView2/wry 会把焦点丢给页面第一个可聚焦元素，
      // 主界面就是左上角那个图标按钮），在同一个事件循环里立刻抢回来 ——
      // 中间那一瞬间不会被绘制出来，所以用户看不到闪烁。
      if (pendingRestore || Date.now() < watchdogUntil) {
        const target = lastEditable;
        const node = event.target;
        if (target && node instanceof Node && node !== target && !target.contains(node)) {
          restoreNow();
          return;
        }
        if (!target) {
          // 还没有目标（这次激活之前没编辑过东西）→ 顺手记一个，别浪费
          const el = asEditable(node);
          if (el && isVisible(el)) rememberEditable(el);
        }
        return;
      }

      const el = asEditable(event.target);
      if (el && isVisible(el)) rememberEditable(el);
    },
    true,
  );

  document.addEventListener(
    "focusout",
    (event) => {
      const el = asEditable(event.target);
      if (!el || el !== lastEditable) return;
      // 只把光标存下来 —— 失焦后 selectionStart 就读不到了。
      //
      // **这里刻意不清记忆**：切屏 / 最小化 / 隐藏到托盘也会让输入框失焦，而那种失焦
      // 恰恰是我们回来时要恢复的场景。早先版本用 `document.hasFocus()` 在
      // `setTimeout(0)` 里区分「页面内移开」与「整体失焦」，但那一瞬间 hasFocus 的真实值
      // 取决于 WebView2 浏览器进程那边的 IPC 时序 —— 它可能还来不及变 false，
      // 于是切屏被误判成「用户自己移开」，记忆被清掉，**修复就此静默失效**。
      rememberCaret(el);
    },
    true,
  );

  // 用户一有动作就停止看门狗（此后焦点归他管，我们不抢）
  document.addEventListener("pointerdown", stopWatchdog, true);
  document.addEventListener("keydown", stopWatchdog, true);

  window.addEventListener("blur", () => {
    // 失焦就先把「待恢复」标记与守备窗口都置上：`window.focus` 不一定先于
    // 那次「抢焦点」的 `focusin` 到达，靠它开窗口会漏。
    pendingRestore = true;
    startWatchdog();
  });

  window.addEventListener("focus", () => {
    scheduleRestore();
    startWatchdog();
  });

  for (const name of [FOCUSED_EVENT, "tauri://focus", RESTORED_EVENT]) {
    try {
      await listen(name, () => {
        scheduleRestore();
        startWatchdog();
      });
    } catch {
      /* 非 Tauri 环境没有这些事件 */
    }
  }

  // 给宿主（Rust `winfocus` 的激活链末端）主动补一刀用：立即校验一次，并把状态
  // 拼成纯 ASCII 字符串回给诊断日志。与上面几个入口的区别是**不排队、不等事件**。
  // 之所以需要它：WebView2/wry 那边的 `MoveFocus` 是异步的，可能落在我们所有
  // 定时重试之后；由 Rust 在激活链末尾再拉一把最稳。
  window.__WEBLAUNCH_RESTORE_FOCUS__ = () => {
    const el = lastEditable;
    // 内嵌页面正显示时键盘该归它，别在这边抢（与 handleWindowFocused 同一判据）
    const allow = !embedShouldBeVisible();
    if (allow && el && isVisible(el) && document.activeElement !== el) refocus(el);
    const active = document.activeElement;
    return (
      `focus=${document.hasFocus() ? 1 : 0} active=${active ? active.tagName : "null"}` +
      ` last=${el ? el.tagName : "null"} allow=${allow ? 1 : 0}`
    );
  };
}

declare global {
  interface Window {
    /** 供 Rust `winfocus` 在激活链末端调用，返回一段可写进诊断日志的状态 */
    __WEBLAUNCH_RESTORE_FOCUS__?: () => string;
  }
}
