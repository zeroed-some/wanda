import { useState, useEffect } from "react";
import { getConfig, updateConfig, getProtonVersions, runDoctor } from "../hooks/useApi";
import type { ConfigDto, ProtonInfo, DoctorReport } from "../types";

export default function SettingsView() {
  const [config, setConfig] = useState<ConfigDto | null>(null);
  const [protonVersions, setProtonVersions] = useState<ProtonInfo[]>([]);
  const [doctor, setDoctor] = useState<DoctorReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [runningDoctor, setRunningDoctor] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    loadData();
  }, []);

  async function loadData() {
    try {
      setLoading(true);
      const [cfg, proton] = await Promise.all([getConfig(), getProtonVersions()]);
      setConfig(cfg);
      setProtonVersions(proton);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleSave() {
    if (!config) return;

    setSaving(true);
    setError(null);
    setSuccess(null);

    try {
      await updateConfig(config);
      setSuccess("Settings saved successfully!");
      setTimeout(() => setSuccess(null), 3000);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  async function handleRunDoctor() {
    setRunningDoctor(true);
    try {
      const report = await runDoctor();
      setDoctor(report);
    } catch (err) {
      setError(String(err));
    } finally {
      setRunningDoctor(false);
    }
  }

  if (loading || !config) {
    return (
      <div>
        <div className="page-header">
          <h1 className="page-title">Settings</h1>
        </div>
        <div style={{ textAlign: "center", padding: "50px" }}>
          <div className="spinner" style={{ margin: "0 auto" }} />
        </div>
      </div>
    );
  }

  return (
    <div>
      <div className="page-header">
        <h1 className="page-title">Settings</h1>
      </div>

      {error && (
        <div
          className="card"
          style={{ backgroundColor: "rgba(248, 113, 113, 0.1)", marginBottom: "20px" }}
        >
          <p style={{ color: "var(--error)" }}>{error}</p>
        </div>
      )}

      {success && (
        <div
          className="card"
          style={{ backgroundColor: "rgba(74, 222, 128, 0.1)", marginBottom: "20px" }}
        >
          <p style={{ color: "var(--success)" }}>{success}</p>
        </div>
      )}

      {/* Steam Settings */}
      <div className="card">
        <h2 className="card-title" style={{ marginBottom: "20px" }}>
          Steam
        </h2>

        <div className="form-group">
          <label className="form-label">Steam Installation Path (leave empty for auto-detect)</label>
          <input
            type="text"
            className="form-input"
            value={config.steam_path || ""}
            onChange={(e) => setConfig({ ...config, steam_path: e.target.value || null })}
            placeholder="Auto-detect"
          />
        </div>

        <div className="form-group">
          <label className="form-checkbox">
            <input
              type="checkbox"
              checked={config.scan_flatpak}
              onChange={(e) => setConfig({ ...config, scan_flatpak: e.target.checked })}
            />
            <span>Scan Flatpak Steam installation</span>
          </label>
        </div>
      </div>

      {/* Proton Settings */}
      <div className="card">
        <h2 className="card-title" style={{ marginBottom: "20px" }}>
          Proton
        </h2>

        <div className="form-group">
          <label className="form-label">Preferred Proton Version</label>
          <select
            className="form-input"
            value={config.preferred_proton || ""}
            onChange={(e) => setConfig({ ...config, preferred_proton: e.target.value || null })}
          >
            <option value="">Auto (Recommended)</option>
            {protonVersions.map((v) => (
              <option key={v.name} value={v.name}>
                {v.name}{" "}
                {v.compatibility === "recommended"
                  ? "(Recommended)"
                  : v.compatibility === "unsupported"
                  ? "(Unsupported)"
                  : ""}
              </option>
            ))}
          </select>
        </div>

        {protonVersions.length > 0 && (
          <div style={{ marginTop: "15px" }}>
            <p style={{ fontSize: "0.8rem", color: "var(--text-secondary)", marginBottom: "10px" }}>
              Available Proton versions:
            </p>
            <div style={{ display: "flex", flexWrap: "wrap", gap: "8px" }}>
              {protonVersions.map((v) => (
                <span
                  key={v.name}
                  className={`badge ${
                    v.compatibility === "recommended"
                      ? "badge-success"
                      : v.compatibility === "supported"
                      ? "badge-success"
                      : v.compatibility === "experimental"
                      ? "badge-warning"
                      : "badge-error"
                  }`}
                >
                  {v.name}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* WeMod Settings */}
      <div className="card">
        <h2 className="card-title" style={{ marginBottom: "20px" }}>
          WeMod
        </h2>

        <div className="form-group">
          <label className="form-checkbox">
            <input
              type="checkbox"
              checked={config.auto_update_wemod}
              onChange={(e) => setConfig({ ...config, auto_update_wemod: e.target.checked })}
            />
            <span>Automatically update WeMod</span>
          </label>
        </div>
      </div>

      <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
        {saving ? "Saving..." : "Save Settings"}
      </button>

      {/* Diagnostics */}
      <div className="card" style={{ marginTop: "30px" }}>
        <div className="card-header">
          <h2 className="card-title">Diagnostics</h2>
          <button
            className="btn btn-secondary btn-small"
            onClick={handleRunDoctor}
            disabled={runningDoctor}
          >
            {runningDoctor ? "Running..." : "Run Doctor"}
          </button>
        </div>

        {doctor && (
          <div className="doctor-section">
            <div className="doctor-item">
              <span className={`doctor-status ${doctor.steam_ok ? "ok" : "error"}`} />
              <span>
                Steam: {doctor.steam_ok ? `Found at ${doctor.steam_path}` : "Not found"}
              </span>
            </div>
            <div className="doctor-item">
              <span className={`doctor-status ${doctor.proton_ok ? "ok" : "error"}`} />
              <span>
                Proton: {doctor.proton_ok ? `${doctor.proton_count} versions found` : "Not found"}
              </span>
            </div>
            <div className="doctor-item">
              <span className={`doctor-status ${doctor.prefix_ok ? "ok" : "error"}`} />
              <span>Prefix: {doctor.prefix_ok ? "OK" : "Not initialized"}</span>
            </div>
            <div className="doctor-item">
              <span className={`doctor-status ${doctor.wemod_ok ? "ok" : "error"}`} />
              <span>WeMod: {doctor.wemod_ok ? "Installed" : "Not installed"}</span>
            </div>

            {doctor.issues.length > 0 && (
              <div style={{ marginTop: "15px" }}>
                <p style={{ color: "var(--warning)", marginBottom: "10px" }}>Issues:</p>
                <ul style={{ paddingLeft: "20px" }}>
                  {doctor.issues.map((issue, i) => (
                    <li key={i} style={{ color: "var(--text-secondary)", fontSize: "0.9rem" }}>
                      {issue}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
