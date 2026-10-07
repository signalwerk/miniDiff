// Read the same release manifest as the native app. Local previews have no manifest.
fetch('update.json', {cache: 'no-store'})
  .then(response => response.ok ? response.json() : null)
  .then(manifest => {
    if (manifest && typeof manifest.version === 'string' && manifest.version) {
      document.getElementById('version').textContent = `Version ${manifest.version}`;
    }
  })
  .catch(() => {});
