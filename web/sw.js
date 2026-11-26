// Cache-first service worker for a home-screen run with no network.
//
// `web/build.sh` stamps the Cargo version into the cache name below before it
// copies this file into web/dist, so every build gets its own cache and the
// activate handler drops the ones behind it. Everything here is immutable for
// the life of a version: the page, the JS, the wasm, fonts and icons. The cache
// name is written as a build-time placeholder and must stay the only one.
const CACHE = "__RATRUN_CACHE__";
const PRECACHE = [
  "index.html",
  "game.js",
  "canvas.js",
  "ratrun.js",
  "ratrun_bg.wasm",
  "manifest.webmanifest",
  "icons/icon-192.png",
  "icons/icon-512.png",
  "icons/maskable-512.png",
];

const resolve = (path) => new URL(path, self.registration.scope).href;
const scope = new URL(self.registration.scope).pathname;

self.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // One missing file (an icon the art pass has not delivered, a font this
    // build does not ship) must not fail the whole install.
    await Promise.all(PRECACHE.map(async (path) => {
      try {
        await cache.add(new Request(resolve(path), { cache: "reload" }));
      } catch (error) {
        // Fetched again on first use.
      }
    }));
    await self.skipWaiting();
  })());
});

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(names
      .filter((name) => name.startsWith("ratrun-") && name !== CACHE)
      .map((name) => caches.delete(name)));
    await self.clients.claim();
  })());
});

// Cache-first, and a navigation keeps its query string (`?debug-frames`,
// `?profile=full`) while always reading the cached page.
self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin || !url.pathname.startsWith(scope)) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const navigation = request.mode === "navigate";
    // A navigation to the scope root is the precached index.html.
    const hit = await cache.match(request, { ignoreSearch: navigation })
      ?? (navigation ? await cache.match(resolve("index.html")) : undefined);
    if (hit) return hit;
    try {
      const response = await fetch(request);
      if (response.ok && response.type === "basic") {
        await cache.put(request, response.clone()).catch(() => {});
      }
      return response;
    } catch (error) {
      const page = await cache.match(resolve("index.html"));
      if (page) return page;
      throw error;
    }
  })());
});
