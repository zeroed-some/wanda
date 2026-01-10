// Types matching Rust DTOs

export interface GameInfo {
  app_id: number;
  name: string;
  size: string;
  uses_proton: boolean;
  install_path: string;
}

export interface PrefixInfo {
  name: string;
  path: string;
  wemod_installed: boolean;
  wemod_version: string | null;
  proton_version: string | null;
  health: "healthy" | "needs_repair" | "corrupted" | "not_created";
  issues: string[];
}

export interface ProtonInfo {
  name: string;
  path: string;
  compatibility: "recommended" | "supported" | "experimental" | "unsupported";
  is_ge: boolean;
  is_recommended: boolean;
}

export interface WemodStatus {
  installed: boolean;
  version: string | null;
  update_available: boolean;
  latest_version: string | null;
}

export interface InitStatus {
  initialized: boolean;
  steam_found: boolean;
  proton_found: boolean;
  prefix_exists: boolean;
  wemod_installed: boolean;
}

export interface DoctorReport {
  steam_ok: boolean;
  steam_path: string | null;
  proton_ok: boolean;
  proton_count: number;
  prefix_ok: boolean;
  wemod_ok: boolean;
  issues: string[];
}

export interface ConfigDto {
  steam_path: string | null;
  scan_flatpak: boolean;
  preferred_proton: string | null;
  auto_update_wemod: boolean;
}
