import { invoke } from "@tauri-apps/api/core";
import type {
  GameInfo,
  PrefixInfo,
  ProtonInfo,
  WemodStatus,
  InitStatus,
  DoctorReport,
  ConfigDto,
} from "../types";

// Game API
export async function getGames(): Promise<GameInfo[]> {
  return invoke("get_games");
}

export async function getGame(appId: number): Promise<GameInfo> {
  return invoke("get_game", { appId });
}

export async function launchGame(appId: number, withWemod: boolean): Promise<void> {
  return invoke("launch_game", { appId, withWemod });
}

// Prefix API
export async function getPrefixes(): Promise<PrefixInfo[]> {
  return invoke("get_prefixes");
}

export async function getPrefixHealth(name: string): Promise<PrefixInfo> {
  return invoke("get_prefix_health", { name });
}

export async function repairPrefix(name: string): Promise<void> {
  return invoke("repair_prefix", { name });
}

// Init API
export async function getInitStatus(): Promise<InitStatus> {
  return invoke("get_init_status");
}

export async function initWanda(): Promise<void> {
  return invoke("init_wanda");
}

// Config API
export async function getConfig(): Promise<ConfigDto> {
  return invoke("get_config");
}

export async function updateConfig(config: ConfigDto): Promise<void> {
  return invoke("update_config", { configDto: config });
}

// WeMod API
export async function getWemodStatus(): Promise<WemodStatus> {
  return invoke("get_wemod_status");
}

export async function updateWemod(): Promise<void> {
  return invoke("update_wemod");
}

// Proton API
export async function getProtonVersions(): Promise<ProtonInfo[]> {
  return invoke("get_proton_versions");
}

// Doctor API
export async function runDoctor(): Promise<DoctorReport> {
  return invoke("run_doctor");
}
