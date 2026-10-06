import { useEffect, useRef } from "react";

type AccountHeadAvatarProps = {
  skinDataUrl?: string | null;
  username?: string;
  size?: number;
  className?: string;
};

const STEVE_PIXELS = [
  ["#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a"],
  ["#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a", "#4a321a"],
  ["#4a321a", "#b88764", "#b88764", "#b88764", "#b88764", "#b88764", "#b88764", "#4a321a"],
  ["#4a321a", "#ffffff", "#383082", "#b88764", "#b88764", "#383082", "#ffffff", "#4a321a"],
  ["#b88764", "#b88764", "#965b40", "#965b40", "#965b40", "#965b40", "#b88764", "#b88764"],
  ["#b88764", "#b88764", "#5a301a", "#5a301a", "#5a301a", "#5a301a", "#b88764", "#b88764"],
  ["#4a321a", "#5a301a", "#5a301a", "#5a301a", "#5a301a", "#5a301a", "#5a301a", "#4a321a"],
  ["#4a321a", "#4a321a", "#5a301a", "#5a301a", "#5a301a", "#5a301a", "#4a321a", "#4a321a"],
];

function drawSteveFace(ctx: CanvasRenderingContext2D, width: number, height: number) {
  const pixelW = width / 8;
  const pixelH = height / 8;
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 8; x++) {
      ctx.fillStyle = STEVE_PIXELS[y][x];
      ctx.fillRect(
        Math.floor(x * pixelW),
        Math.floor(y * pixelH),
        Math.ceil(pixelW),
        Math.ceil(pixelH)
      );
    }
  }
}

export function AccountHeadAvatar({
  skinDataUrl,
  username,
  size = 28,
  className = "",
}: AccountHeadAvatarProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;

    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.imageSmoothingEnabled = false;

    // If no custom skin is provided, render the authentic pixelated Steve head instantly
    if (!skinDataUrl) {
      drawSteveFace(ctx, canvas.width, canvas.height);
      return;
    }

    let cancelled = false;
    const img = new Image();

    img.onload = () => {
      if (cancelled) return;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.imageSmoothingEnabled = false;

      // 1. Draw base head face (x: 8, y: 8, w: 8, h: 8)
      ctx.drawImage(img, 8, 8, 8, 8, 0, 0, canvas.width, canvas.height);

      // 2. Draw helm / hair overlay (x: 40, y: 8, w: 8, h: 8)
      try {
        const tempCanvas = document.createElement("canvas");
        tempCanvas.width = 8;
        tempCanvas.height = 8;
        const tempCtx = tempCanvas.getContext("2d");
        if (tempCtx) {
          tempCtx.drawImage(img, 40, 8, 8, 8, 0, 0, 8, 8);
          const hatData = tempCtx.getImageData(0, 0, 8, 8).data;
          let allSolidBlack = true;
          let hasVisiblePixels = false;

          for (let i = 0; i < hatData.length; i += 4) {
            const alpha = hatData[i + 3];
            if (alpha > 10) {
              hasVisiblePixels = true;
              if (hatData[i] > 5 || hatData[i + 1] > 5 || hatData[i + 2] > 5) {
                allSolidBlack = false;
              }
            }
          }

          if (hasVisiblePixels && !allSolidBlack) {
            ctx.drawImage(img, 40, 8, 8, 8, 0, 0, canvas.width, canvas.height);
          }
        }
      } catch {
        ctx.drawImage(img, 40, 8, 8, 8, 0, 0, canvas.width, canvas.height);
      }
    };

    img.onerror = () => {
      if (cancelled) return;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      drawSteveFace(ctx, canvas.width, canvas.height);
    };

    img.src = skinDataUrl;

    return () => {
      cancelled = true;
    };
  }, [skinDataUrl, username, size]);

  return (
    <div
      className={`account-head-avatar ${className}`}
      style={{
        width: `${size}px`,
        height: `${size}px`,
        borderRadius: "6px",
        overflow: "hidden",
        position: "relative",
        flexShrink: 0,
        backgroundColor: "#16181d",
        border: "1px solid rgba(255, 255, 255, 0.14)",
        boxShadow: "inset 0 1px 0 rgba(255, 255, 255, 0.1)",
      }}
    >
      <canvas
        ref={canvasRef}
        width={size * 2} // 2x Retina sharpness
        height={size * 2}
        style={{
          width: "100%",
          height: "100%",
          display: "block",
          imageRendering: "pixelated",
        }}
      />
    </div>
  );
}
