import { invoke } from "@tauri-apps/api/core";

export interface SaveDialogOptions {
  title?: string;
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
}

/**
 * 由后端打开“另存为”对话框，返回用户选择的路径（取消时为 null）
 *
 * 后端只允许 `tool_save_*` 写入经此对话框选出的路径，且每个路径只能使用一次，
 * 因此保存类命令必须使用这里返回的路径，不能自行拼接。
 */
export const pickSavePath = (options: SaveDialogOptions): Promise<string | null> =>
  invoke<string | null>("pick_save_path", { ...options });
