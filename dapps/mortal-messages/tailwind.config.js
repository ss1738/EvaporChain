/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        evap: {
          bg: "#fafafa",
          surface: "#ffffff",
          border: "#e4e4e7",
          cyan: "#0891b2",
          purple: "#7c3aed",
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
      keyframes: {
        pulse_low: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: "0.5" },
        },
      },
      animation: {
        "pulse-low": "pulse_low 1.5s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
