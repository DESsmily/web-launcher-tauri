/**
 * 纯逻辑回归校验：端口解析 / ANSI 清理 / 命令输出地址提取。
 * 运行：pnpm verify
 */
import { normalizeUrl, parsePort, pickUrlFromLines, stripAnsi } from "../src/lib/url-utils.ts";

let passed = 0;
let failed = 0;

function eq(label: string, actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) === JSON.stringify(expected)) {
    passed++;
  } else {
    failed++;
    console.log(`FAIL ${label}: got ${JSON.stringify(actual)}, want ${JSON.stringify(expected)}`);
  }
}

/* parsePort */
eq("显式端口", parsePort("http://127.0.0.1:3000"), 3000);
eq("http 默认端口", parsePort("http://127.0.0.1"), 80);
eq("https 默认端口", parsePort("https://example.com"), 443);
eq("带路径", parsePort("http://localhost:8080/app"), 8080);
eq("空字符串", parsePort(""), null);
eq("非法地址", parsePort("not a url"), null);

/* stripAnsi */
eq("去除颜色", stripAnsi("\u001b[32mready\u001b[39m"), "ready");
eq("去除清屏序列", stripAnsi("\u001b[2K\u001b[1G\u001b[36mVITE\u001b[39m v5"), "VITE v5");

/* normalizeUrl */
eq("去空格", normalizeUrl("  http://127.0.0.1:3000  "), "http://127.0.0.1:3000");

/* pickUrlFromLines */
eq(
  "优先匹配配置端口",
  pickUrlFromLines(
    [
      "  \u001b[32m➜\u001b[39m  \u001b[1mLocal\u001b[22m:   \u001b[36mhttp://localhost:5173/\u001b[39m",
      "  \u001b[32m➜\u001b[39m  \u001b[1mLocal\u001b[22m:   \u001b[36mhttp://127.0.0.1:3080/?token=abc123\u001b[39m",
    ],
    3080,
  ),
  "http://127.0.0.1:3080/?token=abc123",
);
eq(
  "单行带 token 的地址",
  pickUrlFromLines(["server on http://127.0.0.1:3080?token=xxxx"], 3080),
  "http://127.0.0.1:3080?token=xxxx",
);
eq(
  "本机地址优先于局域网",
  pickUrlFromLines(["Network: http://192.168.1.10:5173/", "Local: http://localhost:5173/"], 5173),
  "http://localhost:5173/",
);
eq(
  "无本机地址时回退",
  pickUrlFromLines(["open https://demo.example.com/app now"], 3000),
  "https://demo.example.com/app",
);
eq("0.0.0.0 改写", pickUrlFromLines(["http://0.0.0.0:4173/"], 4173), "http://127.0.0.1:4173/");
eq("非匹配端口仍可用", pickUrlFromLines(["http://127.0.0.1:80"], 3000), "http://127.0.0.1:80");
eq("无地址", pickUrlFromLines(["compiling...", "done in 1.2s"], 3000), null);
eq("去掉尾部标点", pickUrlFromLines(["ready at http://127.0.0.1:5173/."], 5173), "http://127.0.0.1:5173/");

console.log(`passed=${passed} failed=${failed}`);
if (failed > 0) process.exit(1);
