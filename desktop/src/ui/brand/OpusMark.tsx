export function OpusMark({ className = "", size = 32 }: { className?: string; size?: number }) {
  return (
    <img
      src="/brand/opus-woven-o-c3.svg"
      alt="Opus Mark"
      className={className}
      width={size}
      height={size}
      style={{ width: `${size}px`, height: `${size}px`, objectFit: "contain" }}
      draggable={false}
    />
  );
}

export function OpusLockup({ subtitle = "LAUNCHER" }: { subtitle?: string }) {
  return (
    <div className="opus-lockup" aria-label="OPUS LAUNCHER">
      <img
        src="/brand/opus-woven-o-c3.svg"
        alt="Opus Logo"
        className="opus-lockup__mark"
        style={{
          width: "30px",
          height: "30px",
          objectFit: "contain",
          filter: "drop-shadow(0 2px 8px rgba(0, 0, 0, 0.4))",
        }}
        draggable={false}
      />
      <div className="opus-lockup__text">
        <span className="opus-lockup__word">OPUS</span>
        <span className="opus-lockup__sub">{subtitle}</span>
      </div>
    </div>
  );
}
