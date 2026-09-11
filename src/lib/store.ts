import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, LaunchConfig, Store } from "../types";

const DEFAULT_SETTINGS: AppSettings = {
  closeAction: "tray",
  launchTimeoutSecs: 120,
  probeIntervalMs: 500,
};

export const store = reactive<{
  ready: boolean;
  settings: AppSettings;
  configs: LaunchConfig[];
}>({
  ready: false,
  settings: { ...DEFAULT_SETTINGS },
  configs: [],
});

function normalizeConfig(raw: Partial<LaunchConfig>): LaunchConfig {
  return {
    id: raw.id ?? "",
    name: raw.name ?? "",
    url: raw.url ?? "",
    localStart: raw.localStart !== false,
    command: raw.command ?? "",
    cwd: raw.cwd ?? "",
    isDefault: raw.isDefault === true,
    remark: raw.remark ?? "",
  };
}

function applyStore(raw: Store | null | undefined) {
  const settings = raw?.settings ?? ({} as Partial<AppSettings>);
  store.settings = {
    closeAction: settings.closeAction === "exit" ? "exit" : "tray",
    launchTimeoutSecs: Number(settings.launchTimeoutSecs) || DEFAULT_SETTINGS.launchTimeoutSecs,
    probeIntervalMs: Number(settings.probeIntervalMs) || DEFAULT_SETTINGS.probeIntervalMs,
  };
  store.configs = (raw?.configs ?? []).map(normalizeConfig).filter((c) => !!c.id);
  store.ready = true;
}

export function newId(): string {
  const c = globalThis.crypto as Crypto | undefined;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  return `cfg_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

export async function loadStore(): Promise<void> {
  const raw = await invoke<Store>("load_store");
  applyStore(raw);
}

export async function persist(): Promise<void> {
  await invoke("save_store", {
    store: {
      settings: { ...store.settings },
      configs: store.configs.map((c) => ({ ...c })),
    },
  });
}

/** 默认启动在全局唯一：设置某一个后，其余自动关闭 */
export async function upsertConfig(config: LaunchConfig): Promise<void> {
  const next: LaunchConfig = { ...config, id: config.id || newId() };
  const index = store.configs.findIndex((c) => c.id === next.id);
  if (index >= 0) store.configs[index] = next;
  else store.configs.push(next);

  if (next.isDefault) {
    for (const c of store.configs) {
      if (c.id !== next.id) c.isDefault = false;
    }
  }
  await persist();
}

export async function removeConfig(id: string): Promise<void> {
  const index = store.configs.findIndex((c) => c.id === id);
  if (index >= 0) store.configs.splice(index, 1);
  await invoke("stop_service", { id }).catch(() => undefined);
  await persist();
}

export async function toggleDefault(id: string): Promise<void> {
  const target = store.configs.find((c) => c.id === id);
  if (!target) return;
  const nextValue = !target.isDefault;
  for (const c of store.configs) c.isDefault = c.id === id ? nextValue : false;
  await persist();
}

export async function updateSettings(patch: Partial<AppSettings>): Promise<void> {
  Object.assign(store.settings, patch);
  await persist();
  if (patch.closeAction) {
    await invoke("set_close_action", { action: store.settings.closeAction }).catch(() => undefined);
  }
}

export function defaultConfig(): LaunchConfig | undefined {
  return store.configs.find((c) => c.isDefault);
}

export function findConfig(id: string): LaunchConfig | undefined {
  return store.configs.find((c) => c.id === id);
}

/** 当前由本应用托管的 cmd 服务（config id 列表） */
export const running = reactive<{ ids: string[] }>({ ids: [] });

export async function refreshRunning(): Promise<void> {
  try {
    running.ids = await invoke<string[]>("running_services");
  } catch {
    running.ids = [];
  }
}

export function isRunning(id: string): boolean {
  return running.ids.includes(id);
}
