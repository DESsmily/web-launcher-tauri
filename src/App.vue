<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import AppToolbar from "./components/AppToolbar.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import HomeView from "./views/HomeView.vue";
import ConfigEditor from "./views/ConfigEditor.vue";
import SettingsView from "./views/SettingsView.vue";
import LaunchView from "./views/LaunchView.vue";

import {
  activateTab,
  applyPageTitle,
  EMBED_TITLE_EVENT,
  openInBrowser,
  reloadEmbed,
  restartTab,
  startLaunch,
  syncEmbedVisibility,
  TAB_MENU_EVENT,
  tabs,
  ui,
} from "./lib/session";
import { defaultConfig, isRunning, loadStore, refreshRunning, store } from "./lib/store";

const bootError = ref("");

const view = computed(() => ui.view);

onMounted(async () => {
  try {
    await loadStore();
    await invoke("set_close_action", { action: store.settings.closeAction });
    await refreshRunning();

    await listen("service://exit", () => {
      syncRunningFlags();
      void refreshRunning();
    });

    // 内嵌页面的标题由 WebView2 上报，用作标签页名称
    await listen<{ tab: string; title: string }>(EMBED_TITLE_EVENT, (event) => {
      applyPageTitle(event.payload.tab, event.payload.title);
    });

    // 标签页右键菜单（重载 / 重新启动 / 在系统浏览器打开）
    await listen<{ tab: string; action: string }>(TAB_MENU_EVENT, (event) => {
      void handleTabMenu(event.payload);
    });

    const preset = defaultConfig();
    if (preset) {
      await startLaunch(preset);
    } else {
      ui.view = "home";
    }
    await syncEmbedVisibility();
  } catch (error) {
    bootError.value = error instanceof Error ? error.message : String(error);
  }
});

/** 本地服务退出后同步标签页上的「运行中」状态 */
function syncRunningFlags(): void {
  for (const tab of tabs.list) {
    tab.running = isRunning(tab.configId);
  }
}

/**
 * 标签页原生右键菜单动作分发：reload（重载）始终存在；restart 仅当 Rust 端按
 * `can_restart` 判断后才加入菜单里；browser 用系统浏览器打开该标签页地址。
 */
async function handleTabMenu(payload: { tab: string; action: string }): Promise<void> {
  // 用户点击的标签页可能不是当前激活的那一个，先切过去
  if (payload.tab !== tabs.activeId) {
    await activateTab(payload.tab);
  }

  switch (payload.action) {
    case "reload":
      await reloadEmbed();
      break;
    case "restart":
      // 重新启动 = 先 kill 占着端口的 cmd 进程树，再原样重新走一遍启动流程
      // （端口检测 → 执行命令 → 等就绪 → 打开）。底层走 runLaunch，
      // 已经会复用同一个标签页并在回调里刷新 phase / logs。
      await restartTab(payload.tab);
      break;
    case "browser":
      await openInBrowser();
      break;
    default:
      break;
  }
}
</script>

<template>
  <div class="root">
    <div v-if="bootError" class="boot-error">
      <strong>启动器初始化失败</strong>
      <p>{{ bootError }}</p>
    </div>

    <div v-else class="app-shell">
      <AppToolbar />

      <LaunchView v-if="view === 'launch'" />
      <ConfigEditor v-else-if="view === 'editor'" />
      <SettingsView v-else-if="view === 'settings'" />
      <HomeView v-else />
    </div>

    <ConfirmDialog />

    <div v-if="ui.toast" class="toast">{{ ui.toast }}</div>
  </div>
</template>

<style scoped>
.root {
  height: 100%;
}

.boot-error {
  padding: 40px;
  color: var(--danger);
}

.boot-error p {
  color: var(--text-muted);
  user-select: text;
}
</style>
