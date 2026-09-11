export interface LaunchConfig {
  id: string;
  name: string;
  /** 访问地址，例如 http://127.0.0.1:3000 */
  url: string;
  /** 是否需要由本应用通过 cmd 拉起本地服务 */
  localStart: boolean;
  /** 启动命令 */
  command: string;
  /** 命令工作目录 */
  cwd: string;
  /** 是否默认启动（全局唯一） */
  isDefault: boolean;
  /** 备注 */
  remark: string;
}

export type CloseAction = "tray" | "exit";

export interface AppSettings {
  closeAction: CloseAction;
  launchTimeoutSecs: number;
  probeIntervalMs: number;
}

export interface Store {
  settings: AppSettings;
  configs: LaunchConfig[];
}

export interface LogLine {
  text: string;
  stream: "stdout" | "stderr" | "system";
  time: number;
}

export function createEmptyConfig(): LaunchConfig {
  return {
    id: "",
    name: "",
    url: "",
    localStart: true,
    command: "",
    cwd: "",
    isDefault: false,
    remark: "",
  };
}
