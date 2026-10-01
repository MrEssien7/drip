// Shows the exact change to ~/.claude/settings.json, then writes it (after a
// backup) only when the user clicks.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./installer.css";

interface Preview {
  install: boolean; settingsPath: string; hookPath: string; backupPath: string | null;
  diff: string; changed: boolean; fingerprint: string;
}

const $ = (id: string) => document.getElementById(id)!;
const apply = $("apply") as HTMLButtonElement;
let install = new URLSearchParams(location.search).get("mode") !== "uninstall";
let plan: Preview | null = null;
let done = false;

function message(text: string, kind: "" | "ok" | "err" = "") {
  const m = $("msg"); m.textContent = text; m.className = "msg " + kind;
}

function showDiff(diff: string) {
  $("diff").replaceChildren(...diff.split("\n").map(line => {
    const s = document.createElement("span");
    s.className = line.startsWith("+ ") ? "a" : line.startsWith("- ") ? "d" : "c";
    s.textContent = line.startsWith("- ") ? "−" + line.slice(1) : line;
    return s;
  }));
}

async function load() {
  done = false;
  plan = null;
  apply.disabled = true;
  $("title").textContent = install ? "Install Claude Code hooks" : "Uninstall Claude Code hooks";
  $("lede").textContent = install
    ? "Drip adds one hook per event to your Claude Code settings. Your own hooks and settings stay as they are."
    : "Drip removes only the hooks that run drip-hook.exe. Your own hooks and settings stay as they are.";
  $("hook-label").textContent = install ? "Relay (copied here)" : "Relay";
  apply.textContent = install ? "Back up and install" : "Back up and uninstall";
  message("");
  try {
    plan = await invoke<Preview>("hooks_preview", { install });
  } catch (e) {
    $("diff").textContent = "";
    message(String(e), "err");
    return;
  }
  $("settings-path").textContent = plan.settingsPath;
  $("hook-path").textContent = plan.hookPath;
  $("backup-path").textContent = plan.backupPath ?? "None needed: the file doesn't exist yet, so Drip will create it.";
  showDiff(plan.diff);
  if (!plan.changed) {
    message(install ? "The hooks are already installed. Nothing to change." : "No Drip hooks found. Nothing to change.");
    return;
  }
  apply.disabled = false;
}

apply.addEventListener("click", async () => {
  if (done) { await getCurrentWindow().close(); return; }
  if (!plan) return;
  apply.disabled = true;
  try {
    const backup = await invoke<string | null>("hooks_apply", { install, fingerprint: plan.fingerprint });
    done = true;
    message(`${install ? "Installed" : "Uninstalled"}.${backup ? ` Backup saved to ${backup}` : ""}`, "ok");
    apply.textContent = "Close";
    apply.disabled = false;
  } catch (e) {
    message(String(e), "err");
    // The file may have changed: show what is there now.
    setTimeout(load, 50);
  }
});
$("cancel").addEventListener("click", () => getCurrentWindow().close());
document.addEventListener("keydown", e => { if (e.key === "Escape") getCurrentWindow().close(); });
listen<string>("installer-mode", e => { install = e.payload !== "uninstall"; load(); });

load();
