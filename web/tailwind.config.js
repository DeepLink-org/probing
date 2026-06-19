/** @type {import('tailwindcss').Config} */

// Dynamic classes built via format!("bg-{}", colors::PRIMARY) — safelist those tokens.
const COLOR_TOKENS = [
  "blue-600",
  "blue-700",
  "blue-600/30",
  "blue-100",
  "blue-400",
  "blue-500",
  "blue-50",
  "blue-200",
  "gray-100",
  "gray-200",
  "gray-50",
  "gray-900",
  "gray-800",
  "gray-700",
  "gray-600",
  "gray-500",
  "gray-400",
  "slate-900",
  "slate-800",
  "slate-700/30",
  "slate-700",
  "slate-600",
  "slate-500",
  "slate-400",
  "slate-300",
  "slate-100",
  "slate-800/50",
  "indigo-50/30",
  "green-600",
  "green-700",
  "green-50",
  "green-800",
  "green-200",
  "red-600",
  "red-700",
  "red-50",
  "red-800",
  "red-200",
  "yellow-600",
  "yellow-50",
  "yellow-800",
  "violet-100",
  "violet-700",
  "violet-800",
  "emerald-500",
  "emerald-50",
  "emerald-700",
  "emerald-200",
  "amber-500",
  "amber-50",
  "amber-800",
  "amber-200",
  "amber-100",
];

const UTILITY_PREFIXES = [
  "bg",
  "text",
  "border",
  "border-t",
  "border-b",
  "border-r",
  "border-l",
  "border-l-2",
  "ring",
  "ring-1",
  "hover:bg",
  "hover:text",
  "hover:opacity-90",
  "focus:border",
  "focus:ring-offset",
];

const GRADIENT_PREFIXES = ["from", "via", "to"];

const GRADIENT_DIRECTIONS = [
  "bg-gradient-to-b",
  "bg-gradient-to-r",
  "bg-gradient-to-br",
];

function dynamicSafelist() {
  const out = [...GRADIENT_DIRECTIONS];
  for (const prefix of UTILITY_PREFIXES) {
    for (const token of COLOR_TOKENS) {
      out.push(`${prefix}-${token}`);
    }
  }
  for (const prefix of GRADIENT_PREFIXES) {
    for (const token of COLOR_TOKENS) {
      out.push(`${prefix}-${token}`);
    }
  }
  return out;
}

module.exports = {
  content: ["./index.html", "./src/**/*.rs"],
  safelist: dynamicSafelist(),
  theme: {
    extend: {},
  },
  plugins: [],
};
