<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import AppIcon from "./AppIcon.vue";
import {
  activateTab,
  activeTab,
  closeTab,
  goHome,
  goSettings,
  tabLabel,
  tabs,
  toggleLogs,
  ui,
} from "../lib/session";
import { findConfig } from "../lib/store";

/**
 * 无边框窗口的“窗口菜单”顶栏：
 * 左 = 设置 / 配置（图标，悬浮提示），中 = 标签页（居中）+ 加号，
 * 右 = 查看启动日志（图标）+ 最小化 / 最大化 / 关闭。
 * 除按钮与标签外，整条顶栏的空白区域都可拖动窗口（data-tauri-drag-region），
 * 双击空白由 Tauri 的 drag-region 处理器自动切换最大化。
 */

/** 右键标签页：弹出**原生**右键菜单（重载 / 重新启动 / 系统浏览器打开）。
 * 不用 HTML 浮层，因为标签栏下面是内嵌页面的 WebView2 子窗口，浮层伸下去会被挡住。 */
async function openTabMenu(id: string, event: MouseEvent): Promise<void> {
  event.preventDefault();

  const tab = tabs.list.find((item) => item.id === id);
  if (!tab) return;

  const config = findConfig(tab.configId);
  const canRestart = !!config && config.localStart && !!config.command.trim();

  try {
    await invoke("show_tab_menu", { tab: id, canRestart });
  } catch (error) {
    console.error(error);
  }
}

/* --------------------------------- 窗口控制 --------------------------------- */

const maximized = ref(false);
let offResize: (() => void) | null = null;

async function refreshMaximized(): Promise<void> {
  try {
    maximized.value = await invoke<boolean>("window_is_maximized");
  } catch {
    /* 忽略（浏览器预览等场景） */
  }
}

function minimizeWindow(): void {
  void invoke("window_minimize").catch(() => undefined);
}

function toggleMaximize(): void {
  void invoke("window_toggle_maximize").catch(() => undefined);
}

/** 关闭按钮同样走 CloseRequested，让设置里的关闭策略（托盘 / 退出）统一生效 */
function closeWindow(): void {
  void invoke("window_close").catch(() => undefined);
}

onMounted(async () => {
  await refreshMaximized();
  try {
    const unlisten = await listen("tauri://resize", () => void refreshMaximized());
    offResize = unlisten;
  } catch {
    /* 非打包环境（浏览器预览）没有该事件 */
  }
});

onBeforeUnmount(() => {
  offResize?.();
  offResize = null;
});

/* ---------------------------------- 状态 ---------------------------------- */

const isConfigsView = computed(() => ui.view === "home" || ui.view === "editor");
const logsOn = computed(() => !!activeTab()?.showLogs && ui.view === "launch");
</script>

<template>
  <header class="toolbar" data-tauri-drag-region>
    <!-- 左：设置 / 配置 -->
    <div class="zone left" data-tauri-drag-region>
      <button
        class="nav-icon"
        :class="{ active: ui.view === 'settings' }"
        title="设置"
        @click="goSettings()"
      >
        <AppIcon name="settings" :size="17" />
      </button>
      <button
        class="nav-icon"
        :class="{ active: isConfigsView }"
        title="配置"
        @click="goHome()"
      >
        <AppIcon name="list" :size="17" />
      </button>
    </div>

    <!-- 中：标签页（居中） -->
    <div class="zone center">
      <div class="tabs">
        <div
          v-for="item in tabs.list"
          :key="item.id"
          class="tab"
          :class="{ active: item.id === tabs.activeId && ui.view === 'launch' }"
          :title="tabLabel(item)"
          role="button"
          tabindex="0"
          @click="activateTab(item.id)"
          @keydown.enter="activateTab(item.id)"
          @contextmenu="openTabMenu(item.id, $event)"
        >
          <span
            v-if="item.phase !== 'ready'"
            class="tab-state"
            :class="item.phase === 'error' ? 'err' : 'busy'"
          />
          <span class="tab-title">{{ tabLabel(item) }}</span>
          <button class="tab-close" title="关闭标签页" @click.stop="closeTab(item.id)">
            <AppIcon name="close" :size="12" />
          </button>
        </div>

        <button class="tab-add" title="新建标签页（选择配置启动）" @click="goHome()">
          <AppIcon name="plus" :size="15" />
        </button>
      </div>
    </div>

    <!-- 右：查看启动日志 + 窗口控制 -->
    <div class="zone right" data-tauri-drag-region>
      <button
        class="nav-icon"
        :class="{ on: logsOn }"
        title="查看启动日志"
        @click="toggleLogs()"
      >
        <AppIcon name="terminal" :size="17" />
      </button>

      <div class="win-controls">
        <button class="win-btn" title="最小化" @click="minimizeWindow()">
          <AppIcon name="minimize" :size="14" />
        </button>
        <button class="win-btn" :title="maximized ? '还原' : '最大化'" @click="toggleMaximize()">
          <AppIcon :name="maximized ? 'restore' : 'maximize'" :size="13" />
        </button>
        <button class="win-btn close" title="关闭" @click="closeWindow()">
          <AppIcon name="close" :size="15" />
        </button>
      </div>
    </div>
  </header>
</template>
