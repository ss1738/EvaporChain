/**
 * Generative art for NFT cards.
 * Creates deterministic abstract visuals from NFT ID and energy state.
 */

export function generateNftArt(
  canvas: HTMLCanvasElement,
  nftId: number,
  energyPercent: number,
  state: string
) {
  const ctx = canvas.getContext("2d")!;
  const w = canvas.width;
  const h = canvas.height;

  // Seed random from NFT ID
  let seed = nftId * 2654435761;
  const rand = () => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed / 0x7fffffff;
  };

  // Background gradient based on state
  const grad = ctx.createLinearGradient(0, 0, w, h);
  if (state === "Ghost") {
    grad.addColorStop(0, "#374151");
    grad.addColorStop(1, "#1f2937");
  } else if (state === "Grace") {
    grad.addColorStop(0, "#7c2d12");
    grad.addColorStop(1, "#451a03");
  } else {
    // Active — vibrant based on energy
    const hue = (nftId * 37) % 360;
    const sat = 40 + energyPercent * 0.5;
    const light = 15 + energyPercent * 0.2;
    grad.addColorStop(0, `hsl(${hue}, ${sat}%, ${light + 10}%)`);
    grad.addColorStop(1, `hsl(${(hue + 40) % 360}, ${sat}%, ${light}%)`);
  }
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, w, h);

  // Draw geometric shapes
  const shapeCount = 5 + Math.floor(rand() * 8);
  for (let i = 0; i < shapeCount; i++) {
    const x = rand() * w;
    const y = rand() * h;
    const size = 10 + rand() * 60;
    const opacity = state === "Ghost" ? 0.1 : 0.1 + (energyPercent / 100) * 0.4;
    const hue = (nftId * 37 + i * 60) % 360;

    ctx.save();
    ctx.globalAlpha = opacity;
    ctx.translate(x, y);
    ctx.rotate(rand() * Math.PI * 2);

    const shape = Math.floor(rand() * 3);
    if (shape === 0) {
      // Circle
      ctx.beginPath();
      ctx.arc(0, 0, size, 0, Math.PI * 2);
      ctx.fillStyle = `hsl(${hue}, 70%, 60%)`;
      ctx.fill();
    } else if (shape === 1) {
      // Diamond
      ctx.beginPath();
      ctx.moveTo(0, -size);
      ctx.lineTo(size * 0.6, 0);
      ctx.lineTo(0, size);
      ctx.lineTo(-size * 0.6, 0);
      ctx.closePath();
      ctx.fillStyle = `hsl(${hue}, 60%, 50%)`;
      ctx.fill();
    } else {
      // Ring
      ctx.beginPath();
      ctx.arc(0, 0, size, 0, Math.PI * 2);
      ctx.strokeStyle = `hsl(${hue}, 70%, 60%)`;
      ctx.lineWidth = 2 + rand() * 4;
      ctx.stroke();
    }
    ctx.restore();
  }

  // Energy glow effect at bottom
  if (state !== "Ghost") {
    const glowGrad = ctx.createLinearGradient(0, h * 0.7, 0, h);
    const glowHue = energyPercent > 50 ? 160 : energyPercent > 20 ? 40 : 0;
    glowGrad.addColorStop(0, "transparent");
    glowGrad.addColorStop(1, `hsla(${glowHue}, 80%, 50%, ${energyPercent / 300})`);
    ctx.fillStyle = glowGrad;
    ctx.fillRect(0, 0, w, h);
  }

  // Ghost overlay
  if (state === "Ghost") {
    ctx.fillStyle = "rgba(0,0,0,0.4)";
    ctx.fillRect(0, 0, w, h);
    ctx.font = `${w * 0.3}px serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillStyle = "rgba(255,255,255,0.15)";
    ctx.fillText("👻", w / 2, h / 2);
  }
}
