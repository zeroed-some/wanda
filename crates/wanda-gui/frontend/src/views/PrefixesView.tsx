import { useState, useEffect } from "react";
import { getPrefixes, repairPrefix, getWemodStatus, updateWemod } from "../hooks/useApi";
import type { PrefixInfo, WemodStatus } from "../types";

export default function PrefixesView() {
  const [prefixes, setPrefixes] = useState<PrefixInfo[]>([]);
  const [wemodStatus, setWemodStatus] = useState<WemodStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [repairing, setRepairing] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadData();
  }, []);

  async function loadData() {
    try {
      setLoading(true);
      const [prefixList, wemod] = await Promise.all([getPrefixes(), getWemodStatus()]);
      setPrefixes(prefixList);
      setWemodStatus(wemod);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleRepair(name: string) {
    setRepairing(name);
    try {
      await repairPrefix(name);
      await loadData();
    } catch (err) {
      setError(String(err));
    } finally {
      setRepairing(null);
    }
  }

  async function handleUpdateWemod() {
    setUpdating(true);
    try {
      await updateWemod();
      await loadData();
    } catch (err) {
      setError(String(err));
    } finally {
      setUpdating(false);
    }
  }

  function getHealthBadge(health: string) {
    switch (health) {
      case "healthy":
        return <span className="badge badge-success">Healthy</span>;
      case "needs_repair":
        return <span className="badge badge-warning">Needs Repair</span>;
      case "corrupted":
        return <span className="badge badge-error">Corrupted</span>;
      default:
        return <span className="badge">Unknown</span>;
    }
  }

  if (loading) {
    return (
      <div>
        <div className="page-header">
          <h1 className="page-title">Prefixes</h1>
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
        <h1 className="page-title">Prefixes</h1>
      </div>

      {error && (
        <div
          className="card"
          style={{ backgroundColor: "rgba(248, 113, 113, 0.1)", marginBottom: "20px" }}
        >
          <p style={{ color: "var(--error)" }}>{error}</p>
        </div>
      )}

      {/* WeMod Status */}
      <div className="card">
        <div className="card-header">
          <h2 className="card-title">WeMod Status</h2>
          {wemodStatus?.update_available && (
            <button
              className="btn btn-primary btn-small"
              onClick={handleUpdateWemod}
              disabled={updating}
            >
              {updating ? "Updating..." : `Update to ${wemodStatus.latest_version}`}
            </button>
          )}
        </div>
        {wemodStatus ? (
          <div>
            <p>
              <strong>Installed:</strong>{" "}
              {wemodStatus.installed ? (
                <span style={{ color: "var(--success)" }}>Yes</span>
              ) : (
                <span style={{ color: "var(--error)" }}>No</span>
              )}
            </p>
            {wemodStatus.version && (
              <p>
                <strong>Version:</strong> {wemodStatus.version}
              </p>
            )}
            {wemodStatus.update_available && (
              <p style={{ color: "var(--warning)" }}>
                Update available: {wemodStatus.latest_version}
              </p>
            )}
          </div>
        ) : (
          <p style={{ color: "var(--text-secondary)" }}>Loading...</p>
        )}
      </div>

      {/* Prefix List */}
      <h2 className="card-title" style={{ marginBottom: "15px" }}>
        Wine Prefixes
      </h2>

      {prefixes.length === 0 ? (
        <div className="card">
          <p style={{ color: "var(--text-secondary)" }}>No prefixes found</p>
        </div>
      ) : (
        <div className="prefix-list">
          {prefixes.map((prefix) => (
            <div key={prefix.name} className="prefix-item">
              <div className="prefix-info">
                <h3>
                  {prefix.name} {getHealthBadge(prefix.health)}
                </h3>
                <p>{prefix.path}</p>
                {prefix.proton_version && (
                  <p>
                    <small>Proton: {prefix.proton_version}</small>
                  </p>
                )}
                {prefix.wemod_installed && prefix.wemod_version && (
                  <p>
                    <small>WeMod: {prefix.wemod_version}</small>
                  </p>
                )}
                {prefix.issues.length > 0 && (
                  <ul style={{ marginTop: "10px", color: "var(--warning)", fontSize: "0.8rem" }}>
                    {prefix.issues.map((issue, i) => (
                      <li key={i}>{issue}</li>
                    ))}
                  </ul>
                )}
              </div>
              <div>
                {prefix.health === "needs_repair" && (
                  <button
                    className="btn btn-secondary btn-small"
                    onClick={() => handleRepair(prefix.name)}
                    disabled={repairing === prefix.name}
                  >
                    {repairing === prefix.name ? "Repairing..." : "Repair"}
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
