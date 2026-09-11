<script setup lang="ts">
import { computed, reactive, ref } from "vue";

import AppIcon from "../components/AppIcon.vue";
import { createEmptyConfig, type LaunchConfig } from "../types";
import { goHome, notify, ui } from "../lib/session";
import { findConfig, upsertConfig } from "../lib/store";

const existing = computed<LaunchConfig | undefined>(() =>
  ui.editingId ? findConfig(ui.editingId) : undefined,
);

const draft = reactive<LaunchConfig>({ ...(existing.value ?? createEmptyConfig()) });
const isEdit = computed(() => !!existing.value);

const errors = reactive<Record<string, string>>({});
const saving = ref(false);

function validate(): boolean {
  for (const key of Object.keys(errors)) delete errors[key];

  if (!draft.name.trim()) errors.name = "请填写配置名称";

  const url = draft.url.trim();
  if (!url) {
    errors.url = "请填写访问地址";
  } else if (!/^https?:\/\/[^\s]+$/i.test(url)) {
    errors.url = "访问地址需以 http:// 或 https:// 开头，例如 http://127.0.0.1:3000";
  }

  if (draft.localStart && !draft.command.trim()) {
    errors.command = "开启本地启动后必须填写 cmd 启动命令";
  }

  if (draft.cwd.trim() && !/^[a-zA-Z]:[\\/]|^\\\\|^\//.test(draft.cwd.trim())) {
    errors.cwd = "工作目录需为绝对路径，例如 D:\\projects\\my-app";
  }

  return Object.keys(errors).length === 0;
}

async function save(): Promise<void> {
  if (!validate() || saving.value) return;
  saving.value = true;
  try {
    await upsertConfig({
      ...draft,
      name: draft.name.trim(),
      url: draft.url.trim(),
      command: draft.command.trim(),
      cwd: draft.cwd.trim(),
      remark: draft.remark.trim(),
    });
    notify(isEdit.value ? "配置已更新" : "配置已创建");
    ui.authoring = false;
    goHome();
  } catch (error) {
    notify(String(error));
  } finally {
    saving.value = false;
  }
}

const portSample = computed(() => {
  const url = draft.url.trim();
  const matched = url.match(/:(\d{2,5})(?:\/|$)/);
  if (matched) return matched[1];
  if (/^https:\/\//i.test(url)) return "443";
  if (/^http:\/\//i.test(url)) return "80";
  return "";
});
</script>

<template>
  <div class="scroll-area">
    <div class="page" style="max-width: 680px">
        <div class="page-head">
          <h2>{{ isEdit ? "编辑启动配置" : "新建启动配置" }}</h2>
          <p>填写访问地址；若页面依赖本地服务，可让启动器自动执行 cmd 命令拉起。</p>
        </div>

        <div class="field">
          <label>配置名称 <span style="color: var(--danger)">*</span></label>
          <input
            v-model="draft.name"
            class="input"
            :class="{ invalid: errors.name }"
            placeholder="例如：订单管理后台"
          />
          <span v-if="errors.name" class="error">{{ errors.name }}</span>
        </div>

        <div class="field">
          <label>访问地址 <span style="color: var(--danger)">*</span></label>
          <input
            v-model="draft.url"
            class="input"
            :class="{ invalid: errors.url }"
            placeholder="http://127.0.0.1:3000"
          />
          <span v-if="errors.url" class="error">{{ errors.url }}</span>
          <span v-else class="hint">
            不带端口时按协议推断：http 视为 80，https 视为 443<template v-if="portSample">
              （当前解析端口：{{ portSample }}）</template
            >。
          </span>
        </div>

        <div class="field">
          <label>是否本地启动</label>
          <label class="switch">
            <input v-model="draft.localStart" type="checkbox" />
            <span class="track" />
            <span style="color: var(--text-muted); font-size: 13px">
              {{ draft.localStart ? "开启：启动前先检测端口，未启动则执行下面的 cmd 命令" : "关闭：直接打开访问地址" }}
            </span>
          </label>
        </div>

        <template v-if="draft.localStart">
          <div class="field">
            <label>cmd 启动命令 <span style="color: var(--danger)">*</span></label>
            <textarea
              v-model="draft.command"
              class="textarea"
              :class="{ invalid: errors.command }"
              placeholder="例如：npm run dev"
            />
            <span v-if="errors.command" class="error">{{ errors.command }}</span>
            <span v-else class="hint">
              会在 Windows 的 cmd 中执行，命令输出会被实时捕获；若输出中出现访问地址（如
              http://127.0.0.1:3080?token=xxxx），将直接使用该地址打开页面。
            </span>
          </div>

          <div class="field">
            <label>工作目录（可选）</label>
            <input
              v-model="draft.cwd"
              class="input"
              :class="{ invalid: errors.cwd }"
              placeholder="D:\projects\my-app"
            />
            <span v-if="errors.cwd" class="error">{{ errors.cwd }}</span>
            <span v-else class="hint">留空则在启动器所在目录执行命令。</span>
          </div>
        </template>

        <div class="field">
          <label>是否默认启动</label>
          <label class="switch">
            <input v-model="draft.isDefault" type="checkbox" />
            <span class="track" />
            <span style="color: var(--text-muted); font-size: 13px">
              {{
                draft.isDefault
                  ? "开启：应用启动后自动打开该配置（其他配置的默认启动会自动关闭）"
                  : "关闭：应用启动后进入配置列表手动选择"
              }}
            </span>
          </label>
        </div>

        <div class="field">
          <label>备注（可选）</label>
          <input v-model="draft.remark" class="input" placeholder="给这个配置加点说明" />
        </div>

        <div class="form-actions">
          <button class="btn" @click="goHome()">取消</button>
          <button class="btn primary" :disabled="saving" @click="save()">
            <AppIcon name="check" :size="15" />
            保存
          </button>
        </div>
      </div>
  </div>
</template>
