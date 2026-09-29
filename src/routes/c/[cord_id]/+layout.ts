// A cord's pages are dynamic on its id and load everything over RPC in the
// browser, so none of them is prerendered or server-rendered; the static
// build's SPA fallback page serves the layout and every tab under it.
export const prerender = false;
export const ssr = false;
