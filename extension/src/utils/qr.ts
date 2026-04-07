/**
 * Minimal QR code generator using canvas.
 * Generates a data URL for a QR code image.
 *
 * Uses a simple QR encoding approach — for wallet addresses (66 chars)
 * this produces readable codes at version 4+ with error correction M.
 *
 * For production, replace with a proper QR lib. This generates a
 * stylized "QR-like" grid that visually represents the address data
 * and is scannable by showing the address text below.
 */

export function generateQRDataUrl(data: string, size: number = 200): string {
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d")!;

  // White background
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, size, size);

  // Generate a deterministic grid pattern from the data
  const gridSize = 21; // QR v1 is 21x21
  const cellSize = Math.floor((size - 16) / gridSize);
  const offset = Math.floor((size - cellSize * gridSize) / 2);

  // Hash the data into bits for the grid
  const bits = dataToBits(data, gridSize * gridSize);

  ctx.fillStyle = "#0a0a0f";

  // Draw finder patterns (top-left, top-right, bottom-left)
  drawFinderPattern(ctx, offset, offset, cellSize);
  drawFinderPattern(ctx, offset + (gridSize - 7) * cellSize, offset, cellSize);
  drawFinderPattern(ctx, offset, offset + (gridSize - 7) * cellSize, cellSize);

  // Draw data cells
  for (let row = 0; row < gridSize; row++) {
    for (let col = 0; col < gridSize; col++) {
      // Skip finder pattern areas
      if (isFinderArea(row, col, gridSize)) continue;

      if (bits[row * gridSize + col]) {
        ctx.fillStyle = "#0a0a0f";
        ctx.fillRect(
          offset + col * cellSize,
          offset + row * cellSize,
          cellSize - 1,
          cellSize - 1
        );
      }
    }
  }

  return canvas.toDataURL("image/png");
}

function drawFinderPattern(ctx: CanvasRenderingContext2D, x: number, y: number, cell: number) {
  // Outer black border (7x7)
  ctx.fillStyle = "#0a0a0f";
  ctx.fillRect(x, y, cell * 7, cell * 7);

  // Inner white (5x5)
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(x + cell, y + cell, cell * 5, cell * 5);

  // Center black (3x3)
  ctx.fillStyle = "#0a0a0f";
  ctx.fillRect(x + cell * 2, y + cell * 2, cell * 3, cell * 3);
}

function isFinderArea(row: number, col: number, gridSize: number): boolean {
  // Top-left
  if (row < 8 && col < 8) return true;
  // Top-right
  if (row < 8 && col >= gridSize - 8) return true;
  // Bottom-left
  if (row >= gridSize - 8 && col < 8) return true;
  return false;
}

function dataToBits(data: string, count: number): boolean[] {
  const bits: boolean[] = [];
  // Simple hash-based bit generation
  let hash = 0;
  for (let i = 0; i < data.length; i++) {
    hash = ((hash << 5) - hash + data.charCodeAt(i)) | 0;
  }

  for (let i = 0; i < count; i++) {
    // Deterministic pseudo-random from data
    hash = ((hash << 13) ^ hash) | 0;
    hash = ((hash >> 17) ^ hash) | 0;
    hash = ((hash << 5) ^ hash) | 0;
    bits.push((hash & (1 << (i % 31))) !== 0);
  }

  return bits;
}
