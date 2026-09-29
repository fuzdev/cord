import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import { vite_plugin_fuz_css } from '@fuzdev/fuz_css/vite_plugin_fuz_css.ts';
import { vite_plugin_pkg_json } from '@fuzdev/fuz_ui/vite_plugin_pkg_json.ts';

export default defineConfig({
	plugins: [sveltekit(), vite_plugin_fuz_css(), vite_plugin_pkg_json()],
	server: {
		// in dev, `cordd` (the Rust server) listens on 4061; nginx does this in production
		proxy: {
			// `ws: true` proxies the `/api/ws` WebSocket upgrade through Vite to
			// `cordd`, so the browser connects same-origin — no cross-origin WS or
			// hardcoded server port in the client.
			'/api': { target: 'http://localhost:4061', ws: true },
			'/health': 'http://localhost:4061'
		}
	},
	optimizeDeps: { exclude: ['@fuzdev/blake3-wasm'] }
});
