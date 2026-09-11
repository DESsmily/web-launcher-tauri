/**
 * 与运行环境无关的纯工具函数：ANSI 清理、端口解析、输出地址提取。
 * 不依赖 Tauri API，便于单独进行单元验证。
 */

const ANSI_PATTERN =
  // eslint-disable-next-line no-control-regex
  /[\u001b\u009b][[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]/g;

const URL_PATTERN = /https?:\/\/[^\s"'`<>\\)\]]+/gi;

export function stripAnsi(input: string): string {
  return input.replace(ANSI_PATTERN, "");
}

/** 从访问地址中解析端口：http://127.0.0.1:3000 -> 3000，http://127.0.0.1 -> 80 */
export function parsePort(rawUrl: string): number | null {
  const value = (rawUrl ?? "").trim();
  if (!value) return null;
  try {
    const url = new URL(value);
    if (url.port) {
      const port = Number(url.port);
      return Number.isFinite(port) && port > 0 ? port : null;
    }
    if (url.protocol === "https:") return 443;
    if (url.protocol === "http:") return 80;
    return null;
  } catch {
    const matched = value.match(/:(\d{1,5})(?:\/|$)/);
    return matched ? Number(matched[1]) : null;
  }
}

export function normalizeUrl(rawUrl: string): string {
  return (rawUrl ?? "").trim().replace(/\s+/g, "");
}

function isLocalHost(host: string): boolean {
  return /^(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\]|::1)$/i.test(host);
}

/** 0.0.0.0 / :: 是监听地址，浏览器不可直接访问，转换成 127.0.0.1 */
function fixBindAddress(url: string): string {
  const replaced = url.replace(/^(https?:\/\/)0\.0\.0\.0(?=[:/]|$)/i, "$1127.0.0.1");
  return replaced.replace(/^(https?:\/\/)\[::\](?=[:/]|$)/i, "$1[::1]");
}

/**
 * 从命令输出中挑选可访问地址：
 * 1. 优先本机地址（localhost / 127.0.0.1 / 0.0.0.0 / ::1）
 * 2. 优先端口与配置端口一致的地址
 * 3. 否则取最后出现的一个
 */
export function pickUrlFromLines(lines: string[], preferredPort: number | null): string | null {
  const found: string[] = [];
  for (const line of lines) {
    const matched = stripAnsi(line).match(URL_PATTERN);
    if (!matched) continue;
    for (const item of matched) {
      found.push(item.replace(/[.,;:'"）)】\]]+$/, ""));
    }
  }
  if (found.length === 0) return null;

  const local = found.filter((item) => {
    try {
      return isLocalHost(new URL(item).hostname);
    } catch {
      return false;
    }
  });
  const pool = local.length > 0 ? local : found;

  if (preferredPort !== null) {
    const exact = pool.find((item) => parsePort(item) === preferredPort);
    if (exact) return fixBindAddress(exact);
  }
  return fixBindAddress(pool[pool.length - 1]);
}
