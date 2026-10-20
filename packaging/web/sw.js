// Keeps the web version in the browser: the next start does not download it again, and
// it starts without a connection. The build puts its version in place of __VERSION__, so
// a new build is a new service worker that fetches the new files and drops the old ones.
const VERSION = '__VERSION__';
const CACHE = 'paintboard-' + VERSION;
// The app's files. Pages of shared boards (`.../board/<id>`) ask for them relative to
// themselves, so they are matched by name wherever they are asked for.
const FILES = ['index.html', 'paintboard.js', 'paintboard_bg.wasm', 'icon.png'];
const ROOT = new URL('./', self.location).href;

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE)
      .then((cache) => cache.addAll(FILES.map((name) => ROOT + name)))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(
        keys.filter((key) => key.startsWith('paintboard-') && key !== CACHE)
          .map((key) => caches.delete(key)),
      ))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin) {
    return;
  }
  // Pages come from the network while there is one, so a new build shows; offline, the
  // kept page starts the app.
  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request).catch(() => caches.match(ROOT + 'index.html', { cacheName: CACHE })),
    );
    return;
  }
  // The app's files come from the cache of this version. Anything else (the server's
  // boards, assets, health) is left alone.
  const name = url.pathname.split('/').pop();
  if (FILES.includes(name)) {
    event.respondWith(
      caches.match(ROOT + name, { cacheName: CACHE })
        .then((kept) => kept || fetch(request)),
    );
  }
});
