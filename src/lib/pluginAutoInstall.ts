/**
 * 自动安装标记：插件市场「一键安装」期间，插件 WS 注册不应弹确认框，
 * 而由 PluginRegisterDialog 直接 confirm 启用（用户点的是「安装即启用」）。
 *
 * 用模块级变量而非 store：仅在当前窗口的一次交互内有效，刷新即失效。
 */
let autoInstalling = false;

export function beginAutoInstall(): void {
  autoInstalling = true;
}

export function endAutoInstall(): void {
  autoInstalling = false;
}

export function isAutoInstalling(): boolean {
  return autoInstalling;
}
