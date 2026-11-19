/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        dark: {
          950: '#06080f',
          900: '#090d16',
          850: '#0d1322',
          800: '#111827',
          750: '#151e32',
          700: '#1f293d',
          600: '#334155',
        },
        cyan: {
          DEFAULT: '#00f0ff',
          glow: 'rgba(0, 240, 255, 0.15)',
        },
        emerald: {
          DEFAULT: '#00e599',
          glow: 'rgba(0, 229, 153, 0.15)',
        },
        rose: {
          DEFAULT: '#ff3b30',
          glow: 'rgba(255, 59, 48, 0.15)',
        },
        amber: {
          DEFAULT: '#ffb800',
          glow: 'rgba(255, 184, 0, 0.15)',
        },
      },
      fontFamily: {
        sans: ['Inter', 'Cairo', '-apple-system', 'BlinkMacSystemFont', 'sans-serif'],
        mono: ['"JetBrains Mono"', '"SF Mono"', 'monospace'],
        arabic: ['Cairo', 'Inter', 'sans-serif'],
      },
      animation: {
        'pulse-fast': 'pulse 1.2s cubic-bezier(0.4, 0, 0.6, 1) infinite',
        'glow': 'glow 2s ease-in-out infinite alternate',
      },
      keyframes: {
        glow: {
          '0%': { boxShadow: '0 0 5px rgba(0, 240, 255, 0.2)' },
          '100%': { boxShadow: '0 0 20px rgba(0, 240, 255, 0.6)' },
        }
      }
    },
  },
  plugins: [],
}
