import { listen } from "@tauri-apps/api/event";
import type { Directive } from "vue";

import { RESTORED_EVENT } from "./session";

/**
 * 悬停态类名，用来**替代** CSS `:hover`。
 *
 * 为什么不用 `:hover`：窗口隐藏到托盘（或最小化）时，鼠标已经离开窗口，但 WebView2
 * 收不到 `mouseleave`，浏览器内部的悬停状态会**粘在离开前那一刻**。还原窗口后，
 * 顶栏那三个窗口按钮（尤其是关闭按钮的红色背景）就一直是悬停样式，直到鼠标真正
 * 动一下才会刷新。
 *
 * 用 JS 自己维护的类名就没有这个问题：窗口还原时把 `is-hover` 全部摘掉即可，
 * 与浏览器内部的命中测试无关。
 */
export const HOVER_CLASS = "is-hover";

interface HoverHandlers {
  enter: () => void;
  leave: () => void;
}

const bound = new WeakMap<HTMLElement, HoverHandlers>();

/**
 * `v-hover`：鼠标进出时切换 `is-hover` 类，语义等价于 `:hover`，但可以被一键清除。
 * 用法：`<button v-hover class="win-btn">`，样式侧写 `.win-btn.is-hover { … }`。
 */
export const vHover: Directive<HTMLElement> = {
  mounted(el) {
    const enter = () => el.classList.add(HOVER_CLASS);
    const leave = () => el.classList.remove(HOVER_CLASS);
    el.addEventListener("pointerenter", enter);
    el.addEventListener("pointerleave", leave);
    bound.set(el, { enter, leave });
  },
  unmounted(el) {
    const handlers = bound.get(el);
    if (!handlers) return;
    el.removeEventListener("pointerenter", handlers.enter);
    el.removeEventListener("pointerleave", handlers.leave);
    bound.delete(el);
  },
};

/** 摘掉所有残留的悬停态 */
export function clearHover(): void {
  document
    .querySelectorAll<HTMLElement>(`.${HOVER_CLASS}`)
    .forEach((el) => el.classList.remove(HOVER_CLASS));
}

/**
 * 安装「残留悬停」清理。触发时机都是「鼠标可能已经不在窗口里 / 按钮已经挪走」的时刻：
 *
 * - 窗口失焦（隐藏到托盘、最小化、切到别的应用）——`blur` 在隐藏前就触发，
 *   所以窗口回来时已经是干净状态；
 * - `tauri://resize`——最大化 / 还原会改变工具栏布局，鼠标未必还停在按钮上；
 * - `window://restored`——从托盘还原（兜底，防止某些路径下 blur 没触发）。
 *
 * 只清不清除都无所谓：鼠标下一次真正进入按钮时 `pointerenter` 会重新加上 `is-hover`。
 */
export async function installHoverReset(): Promise<void> {
  const clear = (): void => clearHover();

  window.addEventListener("blur", clear);

  try {
    await listen("tauri://blur", clear);
  } catch {
    /* 非 Tauri 环境（浏览器预览）没有该事件 */
  }

  try {
    await listen("tauri://resize", clear);
  } catch {
    /* 同上 */
  }

  try {
    await listen(RESTORED_EVENT, clear);
  } catch {
    /* 同上 */
  }
}
