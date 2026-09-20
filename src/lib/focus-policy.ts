/**
 * 焦点恢复的**判定**（纯函数，不依赖 DOM 也不依赖 Tauri）。
 *
 * 单独抽出来是因为这里藏着上次那个 bug 的教训：`element.focus()` 在元素**已经是**
 * `document.activeElement` 时是**空操作** —— 而「切屏回来输入框没焦点」的典型现场恰恰是
 * 「DOM 还记得那个输入框，丢的是操作系统级的键盘焦点」。这种情况下再 focus 一次毫无作用，
 * 只会白白触发一轮 focus/blur 事件；该做的是等 Rust 把系统级焦点要回来（见 `winfocus.rs`）。
 *
 * 抽成纯函数之后，这条判定可以脱离浏览器直接跑回归（`pnpm verify`）。
 */

/** 单轮该做什么 */
export type RestoreStep =
  /** 记忆里已经不是它了（用户自己挪开 / 换了元素），或元素已不可见 → 放弃 */
  | "abort"
  /** 页面整体还没拿到焦点 → 什么都不做，等下一轮再看（系统级焦点归 Rust 负责） */
  | "wait"
  /** DOM 焦点确实丢了 → 补回来（并还原光标） */
  | "focus"
  /** 已经达成 → 收工 */
  | "done";

/** 判定所需的现场快照 */
export interface RestoreSnapshot {
  /** 记忆里记的还是不是同一个元素 */
  tracked: boolean;
  /** 元素还在文档里且可见（视图切走了就是 false） */
  visible: boolean;
  /** `document.activeElement` 就是它 */
  activeIsTarget: boolean;
  /** `document.hasFocus()` */
  documentFocused: boolean;
}

/**
 * 判定这一轮该做什么。
 *
 * 四条规则的顺序是有意的：
 * 1. 先看还要不要它（`tracked` / `visible`）—— 用户自己挪开了、或视图切走了就别再抢；
 * 2. DOM 焦点不在它身上 → 补焦点；
 * 3. DOM 焦点在它身上、但整页没焦点 → **只等**。这正是老版本栽的地方：那时会调一次
 *    `focus()`，而它连空操作都算不上「无效」—— 它让代码看起来做了事，实际什么都没发生；
 * 4. 两样都对 → 完成。
 */
export function nextRestoreStep(state: RestoreSnapshot): RestoreStep {
  if (!state.tracked || !state.visible) return "abort";
  if (!state.activeIsTarget) return "focus";
  return state.documentFocused ? "done" : "wait";
}
