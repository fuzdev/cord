// A cord's page is dynamic on its id and loads everything over RPC in the
// browser, so it's neither prerendered nor server-rendered; the static
// build's SPA fallback page serves it.
export const prerender = false;
export const ssr = false;
