import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { FOCUSED_EVENT, RESTORED_EVENT, embedShouldBeVisible } from "./session";

/**
 * 主界面输入框的「焦点保住」。
 *
 * **现象**：在配置 / 设置页把光标放进某个输入框，切屏（Alt+Tab、切到别的窗口或虚拟桌面、
 * 最小化到托盘）再回来，输入框就丢了焦点，得重新点一下才能接着输入。
 *
 * **原因**在多实例 WebView2 上：wry 给**每一个** webview 的 HWND 都挂了窗口子类过程，
 * 收到 `WM_SETFOCUS` 就调 `controller.MoveFocus(Programmatic)`（wry 的 webview2/mod.rs）。
 * 本项目每个标签页都是一个独立的 `embed-*` 子 webview，切标签 / 显示页面时还会对它
 * `set_focus()`。于是窗口重新被激活时，Windows 把键盘焦点还给「线程里最后活动的那个
 * 子窗口」，落点可能是内嵌页面 —— 主界面这一侧的文档随之 blur（`document.activeElement`
 * 退回 `body`、`document.hasFocus()` 变 false），输入框的聚焦态就没了，打字还会打进
 * 内嵌页面，而主界面看起来「怎么点都没反应」。
 *
 * **做法**分两步，缺一不可：
 * 1. 记住主界面最后编辑过的输入元素与光标位置（`focusin` 记，`focusout` 存光标）；
 * 2. 窗口重新获得焦点时先 `focus_webview` 把**系统级**键盘焦点要回主 webview
 *    （这一步 Rust 侧调 `Webview::set_focus()`，DOM 里的 `element.focus()` 做不到 ——
 *    元素本来就已经是 `activeElement`，再 focus 一次是空操作，不会向系统要焦点），
 *    再把焦点与光标补回那个元素。
 *
 * 只在「当前不该由内嵌页面持有焦点」时出手，否则会跟内嵌页面抢键盘。
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

/** 把系统级键盘焦点要回主界面（主 webview） */
async function claimMainWebviewFocus(): Promise<void> {
  try {
    await invoke("focus_webview");
  } catch {
    /* 浏览器预览等非 Tauri 环境忽略 */
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
  if (!el || !isVisible(el)) return;
  // 主界面本来就好好拿着焦点 → 不必插手（也不要把光标挪回去）
  if (document.activeElement === el && document.hasFocus()) return;

  refocus(el);
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
 * 安装焦点恢复。触发时机都是「窗口刚从别的应用 / 托盘 / 最小化状态回来」：
 * - `window://focused`（Rust 的 `WindowEvent::Focused(true)` 转发，最可靠）；
 * - `tauri://focus`（框架自带的窗口焦点事件，做冗余）；
 * - `window focus`（WebView2 自己重新拿到键盘焦点时）。
 */
export async function installFocusRestore(): Promise<void> {
  document.addEventListener(
    "focusin",
    (event) => {
      const el = asEditable(event.target);
      // 焦点移到非输入元素（按钮、卡片…）说明用户自己挪开了，清掉记忆
      rememberEditable(el && isVisible(el) ? el : null);
    },
    true,
  );

  document.addEventListener(
    "focusout",
    (event) => {
      const el = asEditable(event.target);
      if (!el || el !== lastEditable) return;
      // 先把光标存下来 —— 失焦后 selectionStart 就读不到了
      rememberCaret(el);

      window.setTimeout(() => {
        if (lastEditable !== el) return;
        if (!el.isConnected) {
          rememberEditable(null);
          return;
        }
        // 切屏 / 最小化 / 隐藏到托盘：页面整体失焦，**保留**记忆好在回来时补焦点；
        // 页面内部主动移开（点到空白处）：清掉，免得下次回来乱抢焦点
        if (document.hasFocus()) rememberEditable(null);
      }, 0);
    },
    true,
  );

  window.addEventListener("focus", () => scheduleRestore());

  for (const name of [FOCUSED_EVENT, "tauri://focus", RESTORED_EVENT]) {
    try {
      await listen(name, () => scheduleRestore());
    } catch {
      /* 非 Tauri 环境没有这些事件 */
    }
  }
}
