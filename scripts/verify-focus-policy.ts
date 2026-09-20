/**
 * 纯逻辑回归校验：焦点恢复的判定。
 *
 * 这里守的是那次「切屏回来输入框丢焦点」的修复里最容易写错的一环：
 * 丢的是**操作系统级**键盘焦点时，DOM 侧 `document.activeElement` 往往还是那个输入框，
 * 此时调 `element.focus()` 是**空操作** —— 看上去做了事，实际什么都没发生。
 * 所以判定必须是「DOM 焦点也没了才 focus，否则只等」。
 *
 * 运行：pnpm verify
 */
import { nextRestoreStep } from "../src/lib/focus-policy.ts";

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

/** 现场快照，默认值 = 「一切都好」 */
function snapshot(overrides: Partial<Parameters<typeof nextRestoreStep>[0]> = {}) {
  return {
    tracked: true,
    visible: true,
    activeIsTarget: true,
    documentFocused: true,
    ...overrides,
  };
}

/* 已经达成：什么都不用做 */
eq("焦点与光标都在 → 收工", nextRestoreStep(snapshot()), "done");

/* 用户主动挪开 / 视图切走 → 别抢 */
eq("记忆里已换元素 → 放弃", nextRestoreStep(snapshot({ tracked: false })), "abort");
eq("元素已被卸载 → 放弃", nextRestoreStep(snapshot({ visible: false })), "abort");
eq(
  "既不可见又没焦点 → 仍按放弃（不抢）",
  nextRestoreStep(snapshot({ visible: false, activeIsTarget: false, documentFocused: false })),
  "abort",
);

/* DOM 焦点真丢了 → 补回来 */
eq(
  "activeElement 退回 body → 补焦点",
  nextRestoreStep(snapshot({ activeIsTarget: false, documentFocused: false })),
  "focus",
);

/* ★ 关键回归：这一格绝不能是 "focus" —— focus() 是空操作，只能等系统级焦点回来 */
eq(
  "DOM 焦点还在、整页没焦点 → 只等，不空 focus",
  nextRestoreStep(snapshot({ activeIsTarget: true, documentFocused: false })),
  "wait",
);

console.log(`passed=${passed} failed=${failed}`);
if (failed > 0) process.exit(1);
