// Apply only validated appearance choices before the body can paint. No runtime work.
try {
  const appearance = JSON.parse(localStorage.getItem('corpstex.appearance.v1') || '{}');
  if (appearance && typeof appearance === 'object') {
    for (const [key, values] of Object.entries({ palette: ['charcoal', 'midnight', 'forest'], accent: ['gold', 'mint', 'blue'] })) {
      if (values.includes(appearance[key])) document.documentElement.setAttribute(`data-${key}`, appearance[key]);
    }
  }
} catch { /* The welcome surface reports unavailable storage after React mounts. */ }
