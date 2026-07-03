import type { Config } from 'tailwindcss';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        vesper: {
          bg: '#101010',
          panel: '#181818',
          panel2: '#202020',
          border: '#343434',
          text: '#ffffff',
          muted: '#a0a0a0',
          dim: '#6f6f6f',
          yellow: '#ffc799',
          orange: '#ffb86c',
          green: '#99ffe4',
          blue: '#8db4ff',
          red: '#ff8080',
        },
      },
      boxShadow: {
        vesper: '0 24px 80px rgba(0, 0, 0, 0.45)',
      },
      fontFamily: {
        sans: ['Inter', 'ui-sans-serif', 'system-ui', 'sans-serif'],
      },
    },
  },
  plugins: [],
} satisfies Config;
