import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";

import type { LaunchConfig, LogLine } from "../types";
import { launchConfig, type LaunchPhase } from "./launch";
import { findConfig, newId, refreshRunning, store } from "./store";

export type ViewName = "home" | "editor" | "settings" | "launch";

/** 工具栏高度（逻辑像素），内嵌 WebView2 从该高度之下开始渲染 */
export const TOOLBAR_HEIGHT = 40;

/** 内嵌页面铺满工具栏以下的内容区 */
const EMBED_INSETS = { x: 0, y: TOOLBAR_HEIGHT, right: 0, bottom: 0 };

/** 页面标题上报事件（对应 Rust 侧 `embed::TITLE_EVENT`） */
export const EMBED_TITLE_EVENT = "embed://title";

/** 标签页右键菜单动作事件（对应 Rust 侧 `appmenu::EVT_TAB_MENU`），payload 为 `{ tab, action }` */
export const TAB_MENU_EVENT = "menu://tab";

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
let syncChain: Promise<void> = Promise.resolve();

async function runSyncEmbedVisibility(): Promise<void> {
  const tab = activeTab();
  const shouldShow =
    ui.view === "launch" && !!tab && tab.phase === "ready" && !tab.showLogs && !!tab.url;

  try {
    if (!shouldShow || !tab) {
      await invoke("hide_embeds");
      return;
    }
    await invoke("set_embed_insets", { insets: EMBED_INSETS });
    if (!tab.created) {
      await invoke("open_embed", { tab: tab.id, url: tab.url, insets: EMBED_INSETS });
      tab.created = true;
    } else {
      await invoke("show_embed", { tab: tab.id });
    }
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
    await syncEmbedVisibility();
  } catch (error) {
    if (!alive()) return;
    tab.phase = "error";
    tab.error = error instanceof Error ? error.message : String(error);
    tab.message = "启动失败";
    tab.created = false;
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
