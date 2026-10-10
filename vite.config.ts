import { defineConfig } from 'vitest/config';
import { sveltekit } from '@sveltejs/kit/vite';
import { vite_plugin_fuz_css } from '@fuzdev/fuz_css/vite_plugin_fuz_css.ts';
import { vite_plugin_pkg_json } from '@fuzdev/fuz_ui/vite_plugin_pkg_json.ts';
import { make_cross_backend_project } from '@fuzdev/fuz_app/testing/cross_backend/make_cross_backend_project.ts';

// The `cross_backend_rust` project spawns the real `testing_cordd` over HTTP
// and needs Postgres, so it's gated behind `FUZ_TEST_CROSS_BACKEND` and a bare
// `gro test` stays in-process. Run it with `npm run test:cross`.
const cross_backend_projects = process.env.FUZ_TEST_CROSS_BACKEND
	? [
			make_cross_backend_project({
				name: 'cross_backend_rust',
				global_setup: './src/test/cross_backend/global_setup.ts'
			})
		]
	: [];

export default defineConfig({
	plugins: [sveltekit(), vite_plugin_fuz_css(), vite_plugin_pkg_json()],
	test: {
		projects: [
			{
				extends: true,
				test: {
					name: 'unit',
					include: ['src/**/*.test.ts'],
					exclude: ['src/test/cross_backend/**'],
					sequence: { groupOrder: 1 }
				}
			},
			...cross_backend_projects
		]
	},
	server: {
		// Vite watches the whole root, an inotify watch per file, and the Rust `target/`
		// alone can exhaust the user's `max_user_watches` budget
		watch: { ignored: ['**/target/**', '**/.gro/**'] },
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
