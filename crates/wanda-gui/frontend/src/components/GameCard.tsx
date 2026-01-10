import type { GameInfo } from "../types";

interface Props {
  game: GameInfo;
  onLaunch: (appId: number, withWemod: boolean) => void;
  launching: boolean;
}

export default function GameCard({ game, onLaunch, launching }: Props) {
  return (
    <div className="game-card">
      <h3 className="game-name" title={game.name}>
        {game.name}
      </h3>
      <div className="game-meta">
        <span>App ID: {game.app_id}</span>
        <span>{game.size}</span>
      </div>
      <div className="game-actions">
        <button
          className="btn btn-primary btn-small"
          onClick={() => onLaunch(game.app_id, true)}
          disabled={launching}
        >
          {launching ? "Launching..." : "Launch with WeMod"}
        </button>
        <button
          className="btn btn-secondary btn-small"
          onClick={() => onLaunch(game.app_id, false)}
          disabled={launching}
        >
          Launch
        </button>
      </div>
    </div>
  );
}
