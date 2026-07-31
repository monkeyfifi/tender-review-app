import { open } from "@tauri-apps/plugin-dialog";
import { tauriApi } from "./api";
import { renderApp, type FilePicker } from "./app";
import type { SelectedPath } from "./types";

const toSelectedPath = (path: string): SelectedPath => ({
  path,
  name: path.split(/[\\/]/).pop() || "已选择文件",
});

const chooseFile = async (title: string, extensions: string[]): Promise<SelectedPath | null> => {
  const result = await open({ title, multiple: false, directory: false, filters: [{ name: "支持的文件", extensions }] });
  return typeof result === "string" ? toSelectedPath(result) : null;
};

const picker: FilePicker = {
  openTender: () => chooseFile("选择招标文件", ["pdf", "docx"]),
  openBid: () => chooseFile("选择投标文件", ["pdf", "docx"]),
  openBlindBid: () => chooseFile("选择技术暗标 DOCX", ["docx"]),
};

window.addEventListener("DOMContentLoaded", () => {
  const root = document.querySelector<HTMLElement>("#app");
  if (root) renderApp({ root, api: tauriApi, picker });
});
