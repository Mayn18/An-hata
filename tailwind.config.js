/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        background: '#0e1117',
        surface: '#161b22',
        surfaceHover: '#1f242c',
        primary: '#38bdf8',
        secondary: '#94a3b8',
        border: '#30363d',
      }
    },
  },
  plugins: [],
}
