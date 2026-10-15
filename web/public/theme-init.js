// Applies the stored theme and language before first paint to avoid a flash of the wrong theme.
// Kept as an external file so a strict Content-Security-Policy (no inline scripts) works.
try {
  var theme = localStorage.getItem('rustload.theme') || 'system'
  var dark = theme === 'dark' || (theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
  var lang = localStorage.getItem('rustload.lang')
  if (lang) document.documentElement.lang = lang
} catch (e) {}
