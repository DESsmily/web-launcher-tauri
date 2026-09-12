<script setup lang="ts">
import { computed } from "vue";

import AppIcon from "../components/AppIcon.vue";
import type { LaunchConfig } from "../types";
import { goEditor, notify, previewConfig, startLaunch, stopConfig, ui } from "../lib/session";
import { isRunning, removeConfig, store, toggleDefault } from "../lib/store";

const configs = computed(() => store.configs);

function initial(config: LaunchConfig): string {
  const name = (config.name || config.url || "?").trim();
  return name.slice(0, 1).toUpperCase();
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
    <div class="page">
      <!-- 标题与「新增配置」两端对齐 -->
      <div class="page-head row">
        <div>
          <h2>启动配置</h2>
          <p>
            共 {{ configs.length }} 个配置。「启动」会先拉起本地服务再打开页面；
            预览图标不启动服务，直接打开页面。
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

      <div v-else class="list">
        <article
          v-for="config in configs"
          :key="config.id"
          class="card"
          :class="{ 'is-default': config.isDefault }"
        >
          <div class="avatar">{{ initial(config) }}</div>

          <div class="info">
            <div class="name-row">
              <span class="name">{{ config.name || "未命名配置" }}</span>
              <span v-if="config.isDefault" class="tag primary">
                <AppIcon name="star" :size="11" />默认启动
              </span>
              <span class="tag">{{ config.localStart ? "本地启动" : "直接访问" }}</span>
              <span v-if="isRunning(config.id)" class="tag success">运行中</span>
            </div>
            <div class="url">
              {{ config.url }}
              <template v-if="config.localStart && config.command">
                <span style="color: var(--text-faint)"> · </span>
                <span title="启动命令">{{ config.command }}</span>
              </template>
            </div>
          </div>

          <div class="actions">
            <!-- 本地启动的配置：由本应用拉起的服务在运行时，启动按钮变停止 -->
            <template v-if="config.localStart">
              <button
                v-if="isRunning(config.id)"
                class="btn sm"
                title="停止由本应用拉起的本地服务"
                @click="stopConfig(config.id)"
              >
                <AppIcon name="stop" :size="12" />
                停止
              </button>
              <button
                v-else
                class="btn primary sm"
                title="启动本地服务并打开页面"
                @click="startLaunch(config)"
              >
                <AppIcon name="play" :size="12" />
                启动
              </button>
            </template>

            <!-- 预览：不拉起本地服务，直接打开页面 -->
            <button
              class="btn icon sm"
              v-if="isRunning(config.id) || !config.localStart"
              title="预览页面（不启动本地服务）"
              @click="previewConfig(config)"
            >
              <AppIcon name="eye" :size="14" />
            </button>

            <button
              class="btn icon sm"
              :title="config.isDefault ? '取消默认启动' : '设为默认启动'"
              @click="toggleDefault(config.id)"
            >
              <AppIcon :name="config.isDefault ? 'star-fill' : 'star'" :size="14" />
            </button>
            <button class="btn icon sm" title="编辑" @click="goEditor(config.id)">
              <AppIcon name="edit" :size="14" />
            </button>
            <button class="btn icon sm danger" title="删除" @click="requestDelete(config)">
              <AppIcon name="trash" :size="14" />
            </button>
          </div>
        </article>
      </div>
    </div>
  </div>
</template>
