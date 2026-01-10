import { useEffect, useState } from "react";
import { Routes, Route, NavLink, useNavigate } from "react-router-dom";
import { getInitStatus } from "./hooks/useApi";
import type { InitStatus } from "./types";
import GamesView from "./views/GamesView";
import SettingsView from "./views/SettingsView";
import PrefixesView from "./views/PrefixesView";
import SetupView from "./views/SetupView";

function App() {
  const [initStatus, setInitStatus] = useState<InitStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();

  useEffect(() => {
    checkInit();
  }, []);

  async function checkInit() {
    try {
      const status = await getInitStatus();
      setInitStatus(status);
      if (!status.initialized) {
        navigate("/setup");
      }
    } catch (err) {
      console.error("Failed to check init status:", err);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return (
      <div className="setup-container">
        <div className="spinner" />
      </div>
    );
  }

  // Show setup if not initialized
  if (!initStatus?.initialized) {
    return (
      <Routes>
        <Route path="*" element={<SetupView onComplete={() => checkInit()} />} />
      </Routes>
    );
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="sidebar-header">
          <h1>WANDA</h1>
          <p>WeMod on Linux</p>
        </div>
        <nav>
          <ul className="nav-list">
            <li className="nav-item">
              <NavLink
                to="/"
                className={({ isActive }) => `nav-link ${isActive ? "active" : ""}`}
                end
              >
                Games
              </NavLink>
            </li>
            <li className="nav-item">
              <NavLink
                to="/prefixes"
                className={({ isActive }) => `nav-link ${isActive ? "active" : ""}`}
              >
                Prefixes
              </NavLink>
            </li>
            <li className="nav-item">
              <NavLink
                to="/settings"
                className={({ isActive }) => `nav-link ${isActive ? "active" : ""}`}
              >
                Settings
              </NavLink>
            </li>
          </ul>
        </nav>
      </aside>
      <main className="main-content">
        <Routes>
          <Route path="/" element={<GamesView />} />
          <Route path="/prefixes" element={<PrefixesView />} />
          <Route path="/settings" element={<SettingsView />} />
          <Route path="/setup" element={<SetupView onComplete={() => checkInit()} />} />
        </Routes>
      </main>
    </div>
  );
}

export default App;
