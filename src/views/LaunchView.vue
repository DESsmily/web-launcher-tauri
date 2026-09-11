<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";

import AppIcon from "../components/AppIcon.vue";
import {
  activeTab,
  goHome,
  openInBrowser,
  retryLaunch,
  stopCurrentService,
  toggleLogs,
} from "../lib/session";

const tab = computed(() => activeTab());
const logs = computed(() => tab.value?.logs ?? []);

const phaseLabel = computed(() => {
  switch (tab.value?.phase) {
    case "checking":
      return "检测本地端口";
    case "starting":
      return "正在执行启动命令";
    case "waiting":
      return "等待服务就绪";
    case "ready":
      return "已就绪";
    case "error":
      return "启动失败";
    default:
      return "准备中";
  }
});

const showStatusPanel = computed(
  () => !tab.value || tab.value.phase !== "ready" || tab.value.showLogs,
);

const logBox = ref<HTMLElement | null>(null);

async function scrollLogsToEnd(): Promise<void> {
  await nextTick();
  const box = logBox.value;
  if (box) box.scrollTop = box.scrollHeight;
}

watch(() => logs.value.length, scrollLogsToEnd);
onMounted(scrollLogsToEnd);
</script>

<template>
  <div class="launch-stage">
    <!-- 页面就绪且未查看日志时这里留空，由内嵌 WebView2 铺满整块区域 -->
    <div v-if="showStatusPanel" class="status-panel">
      <template v-if="tab">
        <div class="status-head">
          <div v-if="tab.phase === 'error'" class="dot bad" />
          <div v-else-if="tab.phase === 'ready'" class="dot ok" />
          <div v-else class="spinner" />
          <div>
            <div class="status-text">{{ phaseLabel }}</div>
            <div class="status-sub">
              {{ tab.error || tab.message || "正在准备 ..." }}
            </div>
          </div>
        </div>

        <div v-if="tab.phase === 'error'" class="error-panel">
          {{ tab.error }}
        </div>

        <div ref="logBox" class="log-box">
          <div v-if="logs.length === 0" style="color: #7c8798">（暂无输出）</div>
          <div v-for="(line, index) in logs" :key="index" class="line" :class="line.stream">
            {{ line.text }}
          </div>
        </div>

        <div style="display: flex; gap: 8px; flex-wrap: wrap">
          <button v-if="tab.phase === 'error'" class="btn primary" @click="retryLaunch()">
            <AppIcon name="refresh" :size="14" />
            重试启动
          </button>
          <button v-if="tab.running" class="btn danger" @click="stopCurrentService()">
            <AppIcon name="stop" :size="13" />
            结束本地服务
          </button>
          <button v-if="tab.phase === 'ready'" class="btn" @click="toggleLogs()">
            返回页面
          </button>
          <button v-if="tab.url" class="btn" @click="openInBrowser()">
            <AppIcon name="external" :size="14" />
            在系统浏览器打开
          </button>
        </div>
      </template>

      <div v-else class="empty">
        <div class="icon"><AppIcon name="rocket" :size="26" /></div>
        <h3>还没有打开的页面</h3>
        <p>点击工具栏上的加号选择一个配置启动。</p>
        <button class="btn primary" @click="goHome()">
          <AppIcon name="plus" :size="15" />
          选择配置
        </button>
      </div>
    </div>
  </div>
</template>
