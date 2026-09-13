<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { notify } from "../lib/session";
import { store, updateSettings } from "../lib/store";
import type { CloseAction } from "../types";

const info = ref<{ name: string; version: string; tauri: string }>({
  name: "Web 启动器",
  version: "-",
  tauri: "-",
});

const closeAction = computed({
  get: () => store.settings.closeAction,
  set: (value: CloseAction) => void applyCloseAction(value),
});

async function applyCloseAction(action: CloseAction): Promise<void> {
  await updateSettings({ closeAction: action });
  notify(action === "tray" ? "关闭时最小化到托盘" : "关闭时直接退出应用");
}

function setNumber(key: "launchTimeoutSecs" | "probeIntervalMs", raw: number): void {
  const safe = Number.isFinite(raw) ? Math.round(raw) : 0;
  if (key === "launchTimeoutSecs") {
    void updateSettings({ launchTimeoutSecs: Math.min(Math.max(safe, 5), 3600) });
  } else {
    void updateSettings({ probeIntervalMs: Math.min(Math.max(safe, 150), 10000) });
  }
}

async function openGitHubRepo(): Promise<void> {
  await invoke("open_external", { url: "https://github.com/DESsmily/web-launcher-tauri/releases" });
}



onMounted(async () => {
  try {
    info.value = await invoke("app_info");
  } catch {
    /* ignore */
  }
});
</script>

<template>
  <div class="scroll-area">
    <div class="page" style="max-width: 680px">
      <div class="page-head">
        <h2>设置</h2>
        <p>调整启动器的窗口行为与启动等待策略。</p>
      </div>

      <div class="field">
        <label>关闭操作</label>
        <!-- 滑动分段控制器：左右滑动切换“最小化到托盘”与“关闭应用程序” -->
        <div class="segmented-control" role="radiogroup" aria-label="关闭操作">
          <input id="close-tray" v-model="closeAction" type="radio" value="tray" />
          <label for="close-tray" class="segment" :class="{ active: closeAction === 'tray' }">
            最小化到托盘
          </label>

          <input id="close-exit" v-model="closeAction" type="radio" value="exit" />
          <label for="close-exit" class="segment" :class="{ active: closeAction === 'exit' }">
            关闭应用程序
          </label>

          <!-- 滑动指示器，随选中项左右移动 -->
          <div class="slider" :class="{ right: closeAction === 'exit' }"></div>
        </div>

        <p class="option-desc">
          {{ closeAction === 'tray'
            ? '点击关闭时隐藏窗口，应用与已启动的本地服务继续在后台运行，可从托盘图标重新打开。'
            : '点击关闭时退出应用，并自动结束所有由启动器通过 cmd 拉起的本地服务进程。'
          }}
        </p>
        <span class="hint">默认：最小化到托盘。</span>
      </div>

      <div class="field">
        <label>启动等待超时（秒）</label>
        <input class="input" type="number" min="5" max="3600" :value="store.settings.launchTimeoutSecs"
          @change="setNumber('launchTimeoutSecs', Number(($event.target as HTMLInputElement).value))" />
        <span class="hint">执行 cmd 启动命令后，等待服务就绪的最长时间，超时后可点击刷新重试。</span>
      </div>

      <div class="field">
        <label>端口探测间隔（毫秒）</label>
        <input class="input" type="number" min="150" max="10000" step="50" :value="store.settings.probeIntervalMs"
          @change="setNumber('probeIntervalMs', Number(($event.target as HTMLInputElement).value))" />
        <span class="hint">检测本地端口是否已监听的时间间隔，数值越小响应越快、开销略高。</span>
      </div>

      <div class="field">
        <label>GitHub 仓库地址</label>
        <div class="git-link" @click="openGitHubRepo">去查看</div>
      </div>

      <div class="field">
        <label>版本</label>
        <div class="version-card">
          <div class="version-row">
            <span>{{ info.name }}</span>
            <strong>v{{ info.version }}</strong>
          </div>
          <div class="version-row muted">
            <span>Tauri</span>
            <span>{{ info.tauri }}</span>
          </div>
          <div class="version-row muted">
            <span>渲染内核</span>
            <span>Microsoft Edge WebView2</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.segmented-control {
  position: relative;
  display: flex;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius);
  background: var(--surface);
  padding: 3px;
  overflow: hidden;
}

/* 隐藏原生单选框，仅保留可访问性语义 */
.segmented-control input[type="radio"] {
  position: absolute;
  opacity: 0;
  width: 0;
  height: 0;
}

.segment {
  position: relative;
  z-index: 1;
  flex: 1;
  text-align: center;
  padding: 9px 0;
  font-size: 13.5px;
  font-weight: 500;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: calc(var(--radius) - 2px);
  transition: color 0.2s ease;
  user-select: none;
}

.segment.active {
  color: var(--primary);
  font-weight: 600;
}

/* 滑动背景指示器：默认位于左侧，选中“关闭应用程序”时移动到右侧 */
.slider {
  position: absolute;
  top: 3px;
  left: 3px;
  width: calc(50% - 3px);
  height: calc(100% - 6px);
  background: var(--primary-soft);
  border-radius: calc(var(--radius) - 2px);
  transition: left 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  z-index: 0;
}

.slider.right {
  left: calc(50%);
}

.option-desc {
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.55;
  margin: 8px 0 0;
}

.version-card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--surface);
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.version-row {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
}

.version-row.muted {
  color: var(--text-muted);
}

.git-link {
  color: var(--primary);
  cursor: pointer;
}
</style>
