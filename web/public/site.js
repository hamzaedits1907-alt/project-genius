// Keeps the download buttons pointing at the newest published installer. If the lookup fails the buttons keep
// the direct links written into the page.
(function () {
  var cfg = window.GENIUS_SITE || {};
  if (cfg.discordUrl) {
    var d = document.getElementById('discord');
    if (d) { d.href = cfg.discordUrl; d.hidden = false; }
  }
  fetch('https://api.github.com/repos/hamzaedits1907-alt/project-genius/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (rel) {
      if (!rel || !rel.assets) return;
      var find = function (re) { return rel.assets.filter(function (a) { return re.test(a.name) && !/\.sha256$/.test(a.name); })[0]; };
      var find2 = function (re) { return rel.assets.filter(function (a) { return re.test(a.name); })[0]; };
      var exe = find(/x64-setup\.exe$/), msi = find(/\.msi$/);
      var set = function (id, asset) { var el = document.getElementById(id); if (el && asset) { el.href = asset.browser_download_url; if (id === 'dl-exe') el.textContent = asset.name; } };
      set('dl-exe', exe); set('dl-msi', msi); set('dl-win', exe);
      var apk = find(/\.apk$/); var al = document.getElementById('dl-apk');
      if (al && apk) { al.href = apk.browser_download_url; al.textContent = 'Download for Android (' + apk.name + ')'; al.hidden = false; }
      var sum = find2(/SHA256|checksums?/i); var sl = document.getElementById('dl-sum'); if (sl && sum) { sl.href = sum.browser_download_url; sl.hidden = false; }
      var note = document.getElementById('dl-note');
      if (note && exe) note.textContent = 'Version ' + rel.tag_name.replace(/^v/, '') + ' · Windows 10/11 · free · you pay only your own AI provider';
    })
    .catch(function () {});
})();
