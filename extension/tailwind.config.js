/** @type {import('tailwindcss').Config} */
export default {
  content: ["./popup.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        evap: {
          bg: "#0a0a0f",
          surface: "#12121a",
          border: "#1e1e2e",
          cyan: "#00f0ff",
          purple: "#8b5cf6",
          ember: "#FF4D00",
          green: "#22c55e",
          red: "#ef4444",
          amber: "#f59e0b",
          ghost: "#6b7280",
        },
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "monospace"],
      },
    },
  },
  plugins: [],
};
