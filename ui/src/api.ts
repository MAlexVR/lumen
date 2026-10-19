import { invoke } from "@tauri-apps/api/core";

export interface AppInfo {
  name: string;
  version: string;
}

export const appInfo = (): Promise<AppInfo> => invoke<AppInfo>("app_info");
