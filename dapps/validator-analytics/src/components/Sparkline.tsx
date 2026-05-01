/** Tiny inline-SVG sparkline. No external chart lib. */
export function Sparkline({
  values,
  width = 120,
  height = 32,
  stroke = "#0891b2",
  fill = "none",
}: {
  values: number[];
  width?: number;
  height?: number;
  stroke?: string;
  fill?: string;
}) {
  if (values.length === 0) {
    return (
      <svg width={width} height={height} aria-label="no data">
        <line x1={0} y1={height / 2} x2={width} y2={height / 2} stroke="#e4e4e7" strokeDasharray="2 2" />
      </svg>
    );
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const step = values.length > 1 ? width / (values.length - 1) : width;
  const points = values
    .map((v, i) => `${(i * step).toFixed(2)},${(height - ((v - min) / range) * height).toFixed(2)}`)
    .join(" ");
  return (
    <svg width={width} height={height} aria-label="sparkline">
      <polyline points={points} fill={fill} stroke={stroke} strokeWidth={1.5} strokeLinejoin="round" />
    </svg>
  );
}
