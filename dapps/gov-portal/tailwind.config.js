/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        evap: {
          bg: "#fafafa",
          surface: "#ffffff",
          border: "#e4e4e7",
          cyan: "#0891b2",
          purple: "#7c3aed",
          violet: "#8b5cf6",
          emerald: "#10b981",
          ember: "#ea580c",
          green: "#16a34a",
          red: "#dc2626",
          amber: "#d97706",
          ghost: "#9ca3af",
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
