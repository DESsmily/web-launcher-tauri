import { invoke } from "@tauri-apps/api/core";

/**
 * 把**系统级**键盘焦点要回主界面（主 webview）。
 *
 * 单独放一个模块是为了避免 `session.ts` 与 `focus.ts` 互相 import 成环 —— 两边都要用它：
 * - `focus.ts`：窗口重新获得焦点时，把焦点从内嵌页面手里抢回来再补到输入框上；
 * - `session.ts`：内嵌页面被隐藏 / 销毁后（切视图、关标签页、LRU 回收），
 *   如果键盘焦点还留在那个已经不可见的页面上，主界面会「点得动、打不了字」。
 *
 * 为什么必须落到 Rust：DOM 层拿不到「键盘焦点在哪个 webview 上」这件事。
 * 一个窗口里有多个 WebView2 实例，Win32 层面它们是**兄弟**子窗口，谁持有系统级键盘焦点
 * 只有 Rust 侧问 Win32 才知道；`element.focus()` 只管文档内部的焦点，对这件事无能为力。
 * 具体怎么抢（以及主界面这一侧为什么刻意不用 `Webview::set_focus()`）见 `winfocus.rs`。
 */
export async function claimMainWebviewFocus(): Promise<void> {
  try {
    await invoke("focus_webview");
  } catch {
    /* 浏览器预览等非 Tauri 环境忽略 */
  }
}
