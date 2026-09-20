import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";

import type { LaunchConfig, LogLine } from "../types";
import { launchConfig, normalizeUrl, type LaunchPhase } from "./launch";
import { findConfig, isRunning, newId, refreshRunning, store } from "./store";
import { claimMainWebviewFocus } from "./webview-focus";

export type ViewName = "home" | "editor" | "settings" | "launch";

/** 工具栏高度（逻辑像素），内嵌 WebView2 从该高度之下开始渲染 */
export const TOOLBAR_HEIGHT = 40;

/** 内嵌页面铺满工具栏以下的内容区 */
const EMBED_INSETS = { x: 0, y: TOOLBAR_HEIGHT, right: 0, bottom: 0 };

/** 页面标题上报事件（对应 Rust 侧 `embed::TITLE_EVENT`） */
export const EMBED_TITLE_EVENT = "embed://title";

/** 标签页右键菜单动作事件（对应 Rust 侧 `appmenu::EVT_TAB_MENU`），payload 为 `{ tab, action }` */
export const TAB_MENU_EVENT = "menu://tab";

/**
 * 窗口从托盘还原事件（对应 Rust 侧 `lib::RESTORED_EVENT`）。
 * 隐藏到托盘时内嵌页面被一并隐藏，还原后需要重新同步一次显隐。
 */
export const RESTORED_EVENT = "window://restored";

/**
 * 窗口重新获得焦点事件（对应 Rust 侧 `lib::FOCUSED_EVENT`）。
 *
 * 多实例 WebView2 下，窗口重新激活时系统可能把键盘焦点还给某个内嵌子 webview，
 * 导致主界面正在编辑的输入框丢焦点。这个事件专门用来把焦点补回来（见 `lib/focus.ts`）。
 */
export const FOCUSED_EVENT = "window://focused";

/**
 * 内嵌页面实例被回收事件（对应 Rust 侧 `embed::EVICTED_EVENT`），payload `{ tab }`。
 *
 * 存活的内嵌 webview 有数量上限（每个页面都是一个独立渲染进程，无上限地堆积会把
 * 主程序一起拖垮）。超限时 Rust 会销毁最久没用过的那个页面，但**标签页保留** ——
 * 前端收到后把该标签页标记成「未创建」，下次切回去按需重建。
 */
export const EMBED_EVICTED_EVENT = "embed://evicted";

export interface ConfirmState {
  title: string;
  message: string;
  confirmText?: string;
  danger?: boolean;
  onConfirm: () => void | Promise<void>;
}

export const ui = reactive<{
  view: ViewName;
  editingId: string;
  authoring: boolean;
  toast: string;
  toastTimer: number;
  confirm: ConfirmState | null;
}>({
  view: "home",
  editingId: "",
  authoring: false,
  toast: "",
  toastTimer: 0,
  confirm: null,
});

/**
 * 一个标签页 = 一个独立的 WebView2 实例（Rust 侧 label 为 `embed-<id>`）。
 * 关闭标签页只销毁页面，不停止该配置的本地服务。
 */
export interface TabSession {
  id: string;
  configId: string;
  /** 配置名称，页面标题尚未上报时用作兜底显示 */
  configName: string;
  url: string;
  /** 真实页面标题（来自 WebView2 的 DocumentTitleChanged） */
  title: string;
  phase: LaunchPhase | "idle";
  message: string;
  error: string;
  logs: LogLine[];
  /** 是否由本应用拉起了本地服务 */
  running: boolean;
  /** 内嵌 webview 是否已创建（决定 open 还是 show） */
  created: boolean;
  /** 是否正在查看启动日志（此时隐藏内嵌页面） */
  showLogs: boolean;
}

export const tabs = reactive<{ list: TabSession[]; activeId: string }>({
  list: [],
  activeId: "",
});

/** 当前激活的标签页 */
export function activeTab(): TabSession | undefined {
  return tabs.list.find((tab) => tab.id === tabs.activeId);
}

/** 按 id 找标签页 */
export function findTab(id: string): TabSession | undefined {
  return tabs.list.find((tab) => tab.id === id);
}

/** 标签页显示名：优先页面标题，其次配置名 */
export function tabLabel(tab: TabSession): string {
  return tab.title || tab.configName || tab.url || "未命名";
}

export function notify(message: string): void {
  ui.toast = message;
  if (ui.toastTimer) window.clearTimeout(ui.toastTimer);
  ui.toastTimer = window.setTimeout(() => {
    ui.toast = "";
    ui.toastTimer = 0;
  }, 2600);
}

export async function syncWindowTitle(): Promise<void> {
  const tab = activeTab();
  const title = tab ? tabLabel(tab) : "Web 启动器";
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTitle(title);
  } catch {
    /* 权限或环境不支持时忽略 */
  }
}

/* --------------------------------- 导航控制 --------------------------------- */

export function goHome(): void {
  ui.view = "home";
  ui.authoring = false;
  void syncEmbedVisibility();
}

export function goSettings(): void {
  ui.view = "settings";
  ui.authoring = false;
  void syncEmbedVisibility();
}

export function goEditor(id?: string): void {
  ui.editingId = id ?? "";
  ui.authoring = !id;
  ui.view = "editor";
  void syncEmbedVisibility();
}

/** 切换到某个标签页并显示它的内嵌页面 */
export async function activateTab(id: string): Promise<void> {
  if (tabs.activeId === id && ui.view === "launch") return;
  tabs.activeId = id;
  ui.view = "launch";
  await syncWindowTitle();
  await syncEmbedVisibility();
}

/* -------------------------------- 内嵌页控制 -------------------------------- */

/**
 * 把内嵌页面的显隐收敛到一处：只有「当前视图是标签页视图 + 该标签已就绪 + 没在看日志」
 * 时才显示对应实例，其余情况一律隐藏（子 webview 是独立 HWND，永远盖在 HTML 之上）。
 */
/**
 * 当前该不该由内嵌页面占据内容区。
 *
 * 「显隐」判定只此一处：`syncEmbedVisibility` 与 `lib/focus.ts`（判断键盘焦点该归谁）
 * 都读它，避免两边条件写歪之后出现「页面藏了但焦点没还回来」这类错位。
 */
export function embedShouldBeVisible(): boolean {
  const tab = activeTab();
  return ui.view === "launch" && !!tab && tab.phase === "ready" && !tab.showLogs && !!tab.url;
}

let syncChain: Promise<void> = Promise.resolve();

async function runSyncEmbedVisibility(): Promise<void> {
  const tab = activeTab();
  const shouldShow = embedShouldBeVisible();

  try {
    if (!shouldShow || !tab) {
      await invoke("hide_embeds");
      // 内嵌页面藏起来之后，键盘焦点可能还留在它身上（子 webview 是独立 HWND，
      // 抢到焦点后并不会因为被藏起来而自动交还）→ 主界面「点得动、打不了字」。
      // 只在真的丢了焦点时才去要，正常切换视图不产生额外动作。
      if (!document.hasFocus()) await claimMainWebviewFocus();
      return;
    }

    await invoke("set_embed_insets", { insets: EMBED_INSETS });

    if (tab.created) {
      try {
        await invoke("show_embed", { tab: tab.id });
        return;
      } catch {
        // 页面实例可能已经被回收（存活数量有上限，见 Rust `embed.rs::MAX_LIVE_EMBEDS`）
        // 或已随标签页销毁 —— 置回「未创建」，下面按需重建一次。
        tab.created = false;
      }
    }

    await invoke("open_embed", { tab: tab.id, url: tab.url, insets: EMBED_INSETS });
    tab.created = true;
  } catch (error) {
    notify(String(error));
  }
}

/** 串行化，避免快速切换视图时多个 invoke 互相覆盖 */
export function syncEmbedVisibility(): Promise<void> {
  syncChain = syncChain.then(runSyncEmbedVisibility, runSyncEmbedVisibility);
  return syncChain;
}

export async function toggleLogs(): Promise<void> {
  const tab = activeTab();
  if (!tab) {
    notify("还没有打开任何页面");
    return;
  }
  // 日志面板只在标签页视图里可见
  if (ui.view !== "launch") {
    ui.view = "launch";
    await syncWindowTitle();
  }
  tab.showLogs = !tab.showLogs;
  await syncEmbedVisibility();
}

export async function reloadEmbed(): Promise<void> {
  const tab = activeTab();
  if (!tab?.created) return;
  try {
    await invoke("reload_embed", { tab: tab.id });
  } catch (error) {
    notify(String(error));
  }
}

export async function openInBrowser(): Promise<void> {
  const url = activeTab()?.url;
  if (!url) return;
  try {
    await invoke("open_external", { url });
  } catch (error) {
    notify(String(error));
  }
}

/**
 * 在标签页对应配置的**工作目录**下打开一个终端。
 *
 * 目录只在「本地启动 + 填了工作目录」时才拿得到：非本地启动的配置、或者没填目录的配置，
 * 都传 `null` 让 Rust 用默认目录（启动器自身的工作目录）打开 —— 这两种情况在用户看来
 * 是一样的「没有目录就默认」。
 */
export async function openTerminalForTab(tabId: string): Promise<void> {
  const config = findConfig(findTab(tabId)?.configId ?? "");
  const cwd = config?.localStart && config.cwd.trim() ? config.cwd.trim() : null;

  try {
    await invoke("open_terminal", { cwd });
    notify(cwd ? `已在 ${cwd} 打开终端` : "已打开终端");
  } catch (error) {
    notify(`打开终端失败：${String(error)}`);
  }
}

export async function stopCurrentService(): Promise<void> {
  const tab = activeTab();
  if (!tab?.configId) return;
  await invoke("stop_service", { id: tab.configId });
  tab.running = false;
  await refreshRunning();
  notify("已停止本地服务");
}

/** 关闭标签页：销毁页面实例，并激活相邻标签 */
export async function closeTab(id: string): Promise<void> {
  const index = tabs.list.findIndex((tab) => tab.id === id);
  if (index < 0) return;

  runTokens.delete(id);
  tabs.list.splice(index, 1);
  await invoke("close_embed", { tab: id }).catch(() => undefined);

  if (tabs.activeId === id) {
    const next = tabs.list[Math.min(index, tabs.list.length - 1)];
    tabs.activeId = next?.id ?? "";
    if (!next && ui.view === "launch") ui.view = "home";
    await syncWindowTitle();
  }
  await syncEmbedVisibility();
}

/** 页面标题变化（由 Rust 的 DocumentTitleChanged 转发） */
export function applyPageTitle(tabId: string, title: string): void {
  const tab = tabs.list.find((item) => item.id === tabId);
  if (!tab) return;
  const next = title.trim();
  if (!next || next === tab.title) return;
  tab.title = next;
  if (tabs.activeId === tabId) void syncWindowTitle();
}

/**
 * 内嵌页面实例被回收（对应 `embed://evicted`）：标签页还在，只是背后的 webview 被销毁了。
 *
 * 被回收的多半是「不活跃」的标签页（Rust 侧按 LRU 挑），但如果正好命中当前显示的那个，
 * 立刻重建一次 —— 否则用户会对着空白等，直到切视图才恢复。
 */
export function applyEmbedEvicted(tabId: string): void {
  const tab = tabs.list.find((item) => item.id === tabId);
  if (!tab) return;
  tab.created = false;
  if (tab.id === tabs.activeId && ui.view === "launch") void syncEmbedVisibility();
}

/* ------------------------------ 启动一个配置 ------------------------------ */

/** 每个标签页各自的启动令牌：后一次启动会让前一次的回调失效 */
const runTokens = new Map<string, number>();

async function runLaunch(tab: TabSession, config: LaunchConfig): Promise<void> {
  const token = (runTokens.get(tab.id) ?? 0) + 1;
  runTokens.set(tab.id, token);
  const alive = () => runTokens.get(tab.id) === token;

  tab.configId = config.id;
  tab.configName = config.name || config.url;
  tab.url = "";
  tab.title = "";
  tab.phase = "checking";
  tab.message = "正在准备启动 ...";
  tab.logs = [];
  tab.error = "";
  tab.running = false;
  tab.showLogs = false;
  // 复用同一个内嵌实例，交由 open_embed 重新导航到新地址
  tab.created = false;

  await syncEmbedVisibility();

  try {
    const outcome = await launchConfig(config, store.settings, {
      onPhase: (phase, message) => {
        if (!alive()) return;
        tab.phase = phase;
        tab.message = message;
      },
      onLog: (line) => {
        if (!alive()) return;
        tab.logs.push(line);
        if (tab.logs.length > 1000) tab.logs.splice(0, tab.logs.length - 1000);
      },
    });

    if (!alive()) return;

    tab.url = outcome.url;
    tab.running = outcome.spawned;
    tab.phase = "ready";
    tab.message = outcome.spawned ? "本地服务已启动" : "页面已就绪";
    // 同步全局「运行中」清单：配置列表的启动 / 停止按钮按它切换
    await refreshRunning();
    await syncEmbedVisibility();
  } catch (error) {
    if (!alive()) return;
    tab.phase = "error";
    tab.error = error instanceof Error ? error.message : String(error);
    tab.message = "启动失败";
    tab.created = false;
    // 失败也可能已经拉起了进程（例如等端口超时），清单要跟上
    await refreshRunning();
    await syncEmbedVisibility();
  }
}

/** 启动配置：同一配置已有标签页时直接复用（失败状态则重试），否则新建标签页 */
export async function startLaunch(config: LaunchConfig): Promise<void> {
  const existing = tabs.list.find((tab) => tab.configId === config.id);

  if (existing) {
    tabs.activeId = existing.id;
    ui.view = "launch";
    await syncWindowTitle();
    if (existing.phase === "error" || existing.phase === "idle") {
      await runLaunch(existing, config);
    } else {
      await syncEmbedVisibility();
    }
    return;
  }

  const created: TabSession = {
    id: newId(),
    configId: config.id,
    configName: config.name || config.url,
    url: "",
    title: "",
    phase: "idle",
    message: "",
    error: "",
    logs: [],
    running: false,
    created: false,
    showLogs: false,
  };

  // 必须取回 reactive 代理再操作：push 进去的是对象的响应式包装，
  // 继续改裸对象不会触发视图更新（启动状态会卡在初始值）。
  const index = tabs.list.push(created) - 1;
  const tab = tabs.list[index]!;
  tabs.activeId = tab.id;
  ui.view = "launch";
  await syncWindowTitle();
  await runLaunch(tab, config);
}

/* ------------------------------- 预览与停止 ------------------------------- */

/**
 * 预览配置：**不拉起本地服务**，直接用内嵌页打开它的访问地址。
 *
 * 「直接访问」型配置只有这一个入口；「本地启动」型也留着它，用于
 * 「服务已经在别处跑着，只想看一眼页面」。所以要跳过整个 launchConfig
 * 流程（端口探测 / 执行命令 / 等就绪），把标签页直接置成 ready。
 */
export async function previewConfig(config: LaunchConfig): Promise<void> {
  const url = normalizeUrl(config.url);
  if (!url) {
    notify("该配置还没有填写有效的访问地址");
    return;
  }

  const existing = tabs.list.find((tab) => tab.configId === config.id);
  if (existing) {
    // 正在跑启动流程的会话先作废令牌，否则它的回调稍后会把状态改回「启动中」
    runTokens.delete(existing.id);
    existing.url = url;
    existing.title = "";
    existing.phase = "ready";
    existing.message = "预览模式（未启动本地服务）";
    existing.error = "";
    existing.showLogs = false;
    // 置回未创建，让 syncEmbedVisibility 走 open_embed 重新导航到这个地址
    existing.created = false;

    tabs.activeId = existing.id;
    ui.view = "launch";
    await syncWindowTitle();
    await syncEmbedVisibility();
    return;
  }

  const created: TabSession = {
    id: newId(),
    configId: config.id,
    configName: config.name || url,
    url,
    title: "",
    phase: "ready",
    message: "预览模式（未启动本地服务）",
    error: "",
    logs: [],
    running: isRunning(config.id),
    created: false,
    showLogs: false,
  };

  // 同 startLaunch：push 之后必须取回代理再操作
  const tab = tabs.list[tabs.list.push(created) - 1]!;
  tabs.activeId = tab.id;
  ui.view = "launch";
  await syncWindowTitle();
  await syncEmbedVisibility();
}

/**
 * 停止某个配置的本地服务。
 *
 * 只停服务、**不关闭它的标签页**，与「关闭标签页只销毁页面、不停服务」保持对称。
 * 停止后 `running` 清单会刷新，配置列表里的按钮自动从「停止」变回「启动」。
 */
export async function stopConfig(configId: string): Promise<void> {
  try {
    await invoke("stop_service", { id: configId });
  } catch (error) {
    notify(`停止失败：${String(error)}`);
    return;
  }

  for (const tab of tabs.list) {
    if (tab.configId === configId) tab.running = false;
  }
  await refreshRunning();
  notify("已停止本地服务");
}

/**
 * 重新启动某个标签页：先停掉占着端口的本地服务进程，再原样走 runLaunch。
 *
 * 应用场景是用户从标签页右键菜单选了「重新启动」—— Rust 侧已经按
 * `localStart && command` 判断过才会发出该动作，所以这里可以直接操作
 * service 模块。复用同一个标签页（也复用同一份 logs、title），更新 phase
 * 让用户看到「重新启动中…」。
 */
export async function restartTab(tabId: string): Promise<void> {
  const tab = findTab(tabId);
  if (!tab) return;

  const config = findConfig(tab.configId);
  if (!config) {
    notify("该配置已被删除");
    return;
  }
  if (!config.localStart || !config.command.trim()) {
    notify("该配置不是本地启动，无需重启");
    return;
  }

  // 1) 停掉当前的本地服务进程树（taskkill /T /F），避免端口仍被占用
  if (tab.running) {
    try {
      await invoke("stop_service", { id: config.id });
    } catch (error) {
      notify(`停止旧服务失败：${String(error)}`);
    }
  }
  // 同步本地内存中的运行标志，避免界面立即显示「运行中」
  tab.running = false;
  await refreshRunning();

  // 2) 重新走启动流程：端口探测 → 拉起新进程 → 等就绪 → 打开页面
  //    runLaunch 已经在回调里更新 phase/logs，结束后调用 syncEmbedVisibility 显示新页面。
  await runLaunch(tab, config);

  if (tabs.activeId !== tab.id) {
    tabs.activeId = tab.id;
    ui.view = "launch";
    await syncWindowTitle();
    await syncEmbedVisibility();
  }
}

export async function retryLaunch(): Promise<void> {
  const tab = activeTab();
  if (!tab) return;
  const config = findConfig(tab.configId);
  if (!config) {
    notify("该配置已被删除");
    await closeTab(tab.id);
    return;
  }
  await runLaunch(tab, config);
}
