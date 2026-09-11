import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { AppSettings, LaunchConfig, LogLine } from "../types";
import { normalizeUrl, parsePort, pickUrlFromLines, stripAnsi } from "./url-utils";

export { normalizeUrl, parsePort, pickUrlFromLines, stripAnsi };

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/* ---------------------------------- 启动流程 ---------------------------------- */

interface LogPayload {
  id: string;
  line: string;
  stream: string;
}

interface ExitPayload {
  id: string;
  code: number | null;
}

export type LaunchPhase = "checking" | "starting" | "waiting" | "ready" | "error";

export interface LaunchContext {
  onPhase: (phase: LaunchPhase, message: string) => void;
  onLog: (line: LogLine) => void;
}

export interface LaunchOutcome {
  url: string;
  /** 是否由本应用拉起了本地服务 */
  spawned: boolean;
}

async function checkPort(port: number): Promise<boolean> {
  if (!Number.isFinite(port) || port <= 0) return false;
  return invoke<boolean>("check_port", { port });
}

/**
 * 启动一个配置：
 * - 未开启本地启动：直接返回配置地址
 * - 开启本地启动：先探测端口，已启动直接访问；未启动则执行 cmd 命令并等待就绪
 * - 命令输出中出现访问地址时，优先使用该地址
 */
export async function launchConfig(
  config: LaunchConfig,
  settings: AppSettings,
  ctx: LaunchContext,
): Promise<LaunchOutcome> {
  const target = normalizeUrl(config.url);
  if (!target) throw new Error("访问地址为空");

  if (!config.localStart) {
    ctx.onPhase("ready", "直接打开配置地址");
    return { url: target, spawned: false };
  }

  const port = parsePort(target);

  ctx.onPhase("checking", port ? `正在检测本地端口 ${port} ...` : "正在检测本地服务 ...");
  if (port !== null && (await checkPort(port))) {
    ctx.onPhase("ready", `本地端口 ${port} 已在运行，直接打开页面`);
    return { url: target, spawned: false };
  }

  const command = (config.command ?? "").trim();
  if (!command) {
    throw new Error("该配置开启了「本地启动」，但未填写 cmd 启动命令");
  }

  let detectedUrl: string | null = null;
  let processExited = false;
  let exitCode: number | null = null;

  const unlisteners: UnlistenFn[] = [];

  const pushLog = (text: string, stream: LogLine["stream"]) => {
    const clean = stripAnsi(text);
    if (!clean.trim()) return;
    ctx.onLog({ text: clean, stream, time: Date.now() });

    if (!detectedUrl) {
      const candidate = pickUrlFromLines([clean], port);
      if (candidate) {
        detectedUrl = candidate;
        ctx.onPhase("waiting", `检测到访问地址：${candidate}`);
      }
    }
  };

  try {
    unlisteners.push(
      await listen<LogPayload>("service://log", (event) => {
        if (event.payload.id !== config.id) return;
        pushLog(event.payload.line, event.payload.stream === "stderr" ? "stderr" : "stdout");
      }),
    );
    unlisteners.push(
      await listen<ExitPayload>("service://exit", (event) => {
        if (event.payload.id !== config.id) return;
        processExited = true;
        exitCode = event.payload.code;
        pushLog(
          `启动命令已退出${event.payload.code === null ? "" : `（退出码 ${event.payload.code}）`}`,
          "system",
        );
      }),
    );

    ctx.onPhase("starting", "正在执行启动命令 ...");
    ctx.onLog({ text: `$ ${command}`, stream: "system", time: Date.now() });
    await invoke<number>("start_service", {
      id: config.id,
      command,
      cwd: (config.cwd ?? "").trim() || null,
    });

    ctx.onPhase("waiting", port ? `等待端口 ${port} 就绪 ...` : "等待服务就绪 ...");

    const startedAt = Date.now();
    const timeoutMs = Math.max(5, settings.launchTimeoutSecs || 120) * 1000;
    const interval = Math.max(150, settings.probeIntervalMs || 500);
    let detectedSince = 0;

    while (Date.now() - startedAt < timeoutMs) {
      await sleep(interval);

      if (detectedUrl) {
        // 输出里出现地址后，给它一点时间真正开始监听
        if (detectedSince === 0) detectedSince = Date.now();
        const readyByPort = port === null || (await checkPort(port));
        if (readyByPort || Date.now() - detectedSince > 4000) {
          ctx.onPhase("ready", `使用命令输出中的地址：${detectedUrl}`);
          return { url: detectedUrl, spawned: true };
        }
        continue;
      }

      if (port !== null && (await checkPort(port))) {
        await sleep(600);
        if (detectedUrl) {
          ctx.onPhase("ready", `使用命令输出中的地址：${detectedUrl}`);
          return { url: detectedUrl, spawned: true };
        }
        ctx.onPhase("ready", `端口 ${port} 已就绪`);
        return { url: target, spawned: true };
      }

      if (processExited) {
        if (port !== null && (await checkPort(port))) {
          ctx.onPhase("ready", `端口 ${port} 已就绪`);
          return { url: target, spawned: true };
        }
        throw new Error(
          `启动命令已退出${exitCode === null ? "" : `（退出码 ${exitCode}）`}，未能检测到可访问的服务`,
        );
      }
    }

    throw new Error(`等待启动超时（${Math.round(timeoutMs / 1000)} 秒），可稍后点击重试`);
  } finally {
    for (const unlisten of unlisteners) {
      try {
        unlisten();
      } catch {
        /* ignore */
      }
    }
  }
}
