<script setup lang="ts">
import { computed } from "vue";

import AppIcon from "../components/AppIcon.vue";
import { vHover } from "../lib/hover";
import type { LaunchConfig } from "../types";
import { goEditor, notify, previewConfig, startLaunch, stopConfig, ui } from "../lib/session";
import { isRunning, removeConfig, store, toggleDefault } from "../lib/store";

const configs = computed(() => store.configs);

function initial(config: LaunchConfig): string {
  const name = (config.name || config.url || "?").trim();
  return name.slice(0, 1).toUpperCase();
}

/**
 * 点击卡片：**本地启动且未在运行**才去启动（拉起服务 + 打开页面），
 * 其余情况（已在运行 / 不是本地启动）直接预览，不碰服务。
 *
 * 卡片内的按钮都必须 `@click.stop`，否则点「编辑」「删除」会顺带触发这里。
 */
function onCardClick(config: LaunchConfig): void {
  if (config.localStart && !isRunning(config.id)) {
    void startLaunch(config);
    return;
  }
  void previewConfig(config);
}

/** 卡片的 title：既说明点击会发生什么，也补全被截断的文字 */
function cardHint(config: LaunchConfig): string {
  if (!config.localStart) return "点击卡片直接打开页面";
  return isRunning(config.id) ? "正在运行，点击卡片预览页面" : "点击卡片启动本地服务并打开页面";
}

function requestDelete(config: LaunchConfig): void {
  ui.confirm = {
    title: "删除配置",
    message: `确定要删除「${config.name || config.url}」吗？该操作不可恢复。`,
    confirmText: "删除",
    danger: true,
    onConfirm: async () => {
      await removeConfig(config.id);
      notify("配置已删除");
    },
  };
}
</script>

<template>
  <div class="scroll-area">
    <div class="page" style="max-width: 1080px">
      <!-- 标题与「新增配置」两端对齐 -->
      <div class="page-head row">
        <div>
          <h2>启动配置</h2>
          <p>
            共 {{ configs.length }} 个配置。点击卡片：未启动的「本地启动」配置会先拉起服务再打开页面，
            已启动或「直接访问」的配置直接预览页面。
          </p>
        </div>
        <button class="btn primary" @click="goEditor()">
          <AppIcon name="plus" :size="15" />
          新增配置
        </button>
      </div>

      <div v-if="configs.length === 0" class="empty">
        <div class="icon"><AppIcon name="rocket" :size="26" /></div>
        <h3>还没有任何启动配置</h3>
        <p>添加一个配置，填写访问地址（需要本地服务时再填上 cmd 启动命令）即可一键启动。</p>
        <button class="btn primary" @click="goEditor()">
          <AppIcon name="plus" :size="15" />
          新增配置
        </button>
      </div>

      <div v-else class="cards">
        <article
          v-for="config in configs"
          :key="config.id"
          class="card config-card"
          v-hover
          :class="{
            'is-default': config.isDefault,
            'is-running': isRunning(config.id),
          }"
          role="button"
          tabindex="0"
          :title="cardHint(config)"
          @click="onCardClick(config)"
          @keydown.enter.self="onCardClick(config)"
          @keydown.space.self.prevent="onCardClick(config)"
        >
          <header class="card-head">
            <div class="avatar">{{ initial(config) }}</div>
            <div class="head-text">
              <div class="name">{{ config.name || "未命名配置" }}</div>
              <div class="tags">
                <span v-if="config.isDefault" class="tag primary">
                  <AppIcon name="star" :size="11" />默认启动
                </span>
                <span class="tag">{{ config.localStart ? "本地启动" : "直接访问" }}</span>
                <span v-if="isRunning(config.id)" class="tag success">
                  <span class="live-dot" />运行中
                </span>
              </div>
            </div>
          </header>

          <div class="card-body">
            <div class="meta">
              <AppIcon name="link" :size="12" />
              <span class="ellipsis" :title="config.url">{{ config.url || "（未填写访问地址）" }}</span>
            </div>
            <div v-if="config.localStart && config.command" class="meta">
              <AppIcon name="terminal" :size="12" />
              <span class="ellipsis" :title="config.command">{{ config.command }}</span>
            </div>
          </div>

          <!-- 卡片底部操作条：按钮一律 .stop，避免冒泡到卡片点击 -->
          <footer class="card-foot">
            <div class="primary-action">
              <template v-if="config.localStart">
                <button
                  v-if="isRunning(config.id)"
                  class="btn sm"
                  title="停止由本应用拉起的本地服务"
                  @click.stop="stopConfig(config.id)"
                >
                  <AppIcon name="stop" :size="12" />
                  停止
                </button>
                <button
                  v-else
                  class="btn primary sm"
                  title="启动本地服务并打开页面"
                  @click.stop="startLaunch(config)"
                >
                  <AppIcon name="play" :size="12" />
                  启动
                </button>
              </template>
              <button
                v-else
                class="btn primary sm"
                title="直接打开页面"
                @click.stop="previewConfig(config)"
              >
                <AppIcon name="eye" :size="12" />
                预览
              </button>
            </div>

            <div class="tools">
              <!-- 服务在跑时也留一个「只看页面」的入口 -->
              <button
                v-if="config.localStart && isRunning(config.id)"
                class="btn icon sm"
                title="预览页面（不启动本地服务）"
                @click.stop="previewConfig(config)"
              >
                <AppIcon name="eye" :size="14" />
              </button>
              <button
                class="btn icon sm"
                :title="config.isDefault ? '取消默认启动' : '设为默认启动'"
                @click.stop="toggleDefault(config.id)"
              >
                <AppIcon :name="config.isDefault ? 'star-fill' : 'star'" :size="14" />
              </button>
              <button class="btn icon sm" title="编辑" @click.stop="goEditor(config.id)">
                <AppIcon name="edit" :size="14" />
              </button>
              <button class="btn icon sm danger" title="删除" @click.stop="requestDelete(config)">
                <AppIcon name="trash" :size="14" />
              </button>
            </div>
          </footer>
        </article>
      </div>
    </div>
  </div>
</template>
