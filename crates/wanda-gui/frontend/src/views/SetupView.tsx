import { useState, useEffect } from "react";
import { initWanda, getInitStatus, runDoctor } from "../hooks/useApi";
import type { InitStatus, DoctorReport } from "../types";

interface Props {
  onComplete: () => void;
}

type SetupStep = "checking" | "ready" | "initializing" | "complete" | "error";

export default function SetupView({ onComplete }: Props) {
  const [step, setStep] = useState<SetupStep>("checking");
  const [status, setStatus] = useState<InitStatus | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState("");

  useEffect(() => {
    checkSystem();
  }, []);

  async function checkSystem() {
    try {
      const [initStatus, doctorReport] = await Promise.all([
        getInitStatus(),
        runDoctor(),
      ]);
      setStatus(initStatus);
      setDoctor(doctorReport);

      if (initStatus.initialized) {
        setStep("complete");
        onComplete();
      } else if (doctorReport.steam_ok && doctorReport.proton_ok) {
        setStep("ready");
      } else {
        setStep("error");
      }
    } catch (err) {
      setError(String(err));
      setStep("error");
    }
  }

  async function startSetup() {
    setStep("initializing");
    setProgress("Creating Wine prefix...");

    try {
      await initWanda();
      setStep("complete");
      setTimeout(onComplete, 1500);
    } catch (err) {
      setError(String(err));
      setStep("error");
    }
  }

  return (
    <div className="setup-container">
      <div className="setup-card">
        <div className="setup-icon">🪄</div>
        <h1 className="setup-title">Welcome to WANDA</h1>
        <p className="setup-description">
          Let's set up WeMod for Linux. This will create a Wine prefix and install WeMod.
        </p>

        {step === "checking" && (
          <>
            <div className="spinner" style={{ margin: "20px auto" }} />
            <p>Checking system requirements...</p>
          </>
        )}

        {step === "ready" && (
          <>
            <div className="setup-steps">
              <div className="setup-step">
                <span className={`step-icon ${status?.steam_found ? "done" : "error"}`}>
                  {status?.steam_found ? "✓" : "✕"}
                </span>
                <span>Steam detected{doctor?.steam_path ? ` at ${doctor.steam_path}` : ""}</span>
              </div>
              <div className="setup-step">
                <span className={`step-icon ${status?.proton_found ? "done" : "error"}`}>
                  {status?.proton_found ? "✓" : "✕"}
                </span>
                <span>
                  Proton found ({doctor?.proton_count || 0} version
                  {doctor?.proton_count !== 1 ? "s" : ""})
                </span>
              </div>
              <div className="setup-step">
                <span className="step-icon pending">3</span>
                <span>Create Wine prefix & install WeMod</span>
              </div>
            </div>
            <button className="btn btn-primary" onClick={startSetup}>
              Initialize WANDA
            </button>
          </>
        )}

        {step === "initializing" && (
          <>
            <div className="spinner" style={{ margin: "20px auto" }} />
            <p>{progress || "This may take several minutes..."}</p>
            <p style={{ fontSize: "0.8rem", color: "var(--text-secondary)", marginTop: "10px" }}>
              Installing .NET Framework and WeMod
            </p>
          </>
        )}

        {step === "complete" && (
          <>
            <div style={{ fontSize: "4rem", marginBottom: "20px" }}>✓</div>
            <p style={{ color: "var(--success)" }}>WANDA is ready!</p>
          </>
        )}

        {step === "error" && (
          <>
            <div style={{ fontSize: "4rem", marginBottom: "20px" }}>⚠️</div>
            <p style={{ color: "var(--error)", marginBottom: "20px" }}>
              {error || "Setup failed"}
            </p>
            {doctor?.issues && doctor.issues.length > 0 && (
              <div style={{ textAlign: "left", marginBottom: "20px" }}>
                <p style={{ marginBottom: "10px", color: "var(--text-secondary)" }}>Issues:</p>
                <ul style={{ listStyle: "disc", paddingLeft: "20px" }}>
                  {doctor.issues.map((issue, i) => (
                    <li key={i} style={{ color: "var(--error)", fontSize: "0.9rem" }}>
                      {issue}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <button className="btn btn-secondary" onClick={checkSystem}>
              Retry
            </button>
          </>
        )}
      </div>
    </div>
  );
}
