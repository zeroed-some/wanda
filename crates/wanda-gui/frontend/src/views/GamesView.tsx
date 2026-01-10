import { useState, useEffect } from "react";
import { getGames, launchGame } from "../hooks/useApi";
import type { GameInfo } from "../types";
import GameCard from "../components/GameCard";

export default function GamesView() {
  const [games, setGames] = useState<GameInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [launching, setLaunching] = useState<number | null>(null);

  useEffect(() => {
    loadGames();
  }, []);

  async function loadGames() {
    try {
      setLoading(true);
      const gamesList = await getGames();
      // Sort by name
      gamesList.sort((a, b) => a.name.localeCompare(b.name));
      setGames(gamesList);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }

  async function handleLaunch(appId: number, withWemod: boolean) {
    setLaunching(appId);
    try {
      await launchGame(appId, withWemod);
    } catch (err) {
      setError(String(err));
    } finally {
      setLaunching(null);
    }
  }

  const filteredGames = games.filter((game) =>
    game.name.toLowerCase().includes(search.toLowerCase())
  );

  if (loading) {
    return (
      <div>
        <div className="page-header">
          <h1 className="page-title">Games</h1>
        </div>
        <div style={{ textAlign: "center", padding: "50px" }}>
          <div className="spinner" style={{ margin: "0 auto" }} />
          <p style={{ marginTop: "20px", color: "var(--text-secondary)" }}>Loading games...</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <div className="page-header">
        <h1 className="page-title">Games</h1>
        <button className="btn btn-secondary" onClick={loadGames}>
          Refresh
        </button>
      </div>

      {error && (
        <div
          className="card"
          style={{ backgroundColor: "rgba(248, 113, 113, 0.1)", marginBottom: "20px" }}
        >
          <p style={{ color: "var(--error)" }}>{error}</p>
        </div>
      )}

      <div className="form-group">
        <input
          type="text"
          className="form-input"
          placeholder="Search games..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {filteredGames.length === 0 ? (
        <div className="card" style={{ textAlign: "center" }}>
          <p style={{ color: "var(--text-secondary)" }}>
            {search ? "No games match your search" : "No Proton games found"}
          </p>
        </div>
      ) : (
        <div className="game-grid">
          {filteredGames.map((game) => (
            <GameCard
              key={game.app_id}
              game={game}
              onLaunch={handleLaunch}
              launching={launching === game.app_id}
            />
          ))}
        </div>
      )}

      <p
        style={{
          marginTop: "20px",
          fontSize: "0.8rem",
          color: "var(--text-secondary)",
          textAlign: "center",
        }}
      >
        Showing {filteredGames.length} of {games.length} Proton games
      </p>
    </div>
  );
}
