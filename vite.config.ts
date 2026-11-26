import { defineConfig } from 'vitest/config';
import { loadEnv } from 'vite';
import type { Adapter } from '@sveltejs/kit';
import { sveltekit } from '@sveltejs/kit/vite';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import adapter from '@sveltejs/adapter-static';
import { execSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { svelte_preprocess_mdz } from '@fuzdev/mdz/svelte_preprocess_mdz.ts';
import { svelte_preprocess_fuz_code } from '@fuzdev/fuz_code/svelte_preprocess_fuz_code.ts';
import { vite_plugin_fuz_css } from '@fuzdev/fuz_css/vite_plugin_fuz_css.ts';
import { vite_plugin_pkg_json } from '@fuzdev/fuz_ui/vite_plugin_pkg_json.ts';
import {
	cross_backend_enabled,
	make_cross_backend_project
} from '@fuzdev/fuz_app/testing/cross_backend/make_cross_backend_project.ts';

import { CORD_SITE_DOMAIN, CORD_SITE_PATHS, is_cord_site_path } from './src/lib/cord_site.ts';

// The `cross_backend_rust` project spawns the real `testing_cordd` over HTTP
// (building it and a `pglet_server`), so it's gated behind `FUZ_TEST_CROSS_BACKEND` and a bare
// `gro test` stays in-process. Run it with `npm run test:cross`.
const cross_backend_projects = cross_backend_enabled()
	? [
			make_cross_backend_project({
				name: 'cross_backend_rust',
				global_setup: './src/test/cross_backend/global_setup.ts'
			})
		]
	: [];

const BUILD_DIR = 'build';

// cord.fuz.dev, the static site on GitHub Pages: `404.html` is Pages' fallback,
// and `CNAME` and `.nojekyll` (Jekyll would drop `_app/`) go only in this
// build, so no instance's web root ships them
const create_site_adapter = (): Adapter => {
	const base = adapter({ pages: BUILD_DIR, assets: BUILD_DIR, fallback: '404.html' });
	return {
		...base,
		adapt: async (builder) => {
			await base.adapt(builder);
			writeFileSync(join(BUILD_DIR, 'CNAME'), CORD_SITE_DOMAIN + '\n');
			writeFileSync(join(BUILD_DIR, '.nojekyll'), '');
		}
	};
};

export default defineConfig(({ mode }) => {
	// read as SvelteKit reads it, the process env over the env files, so the
	// config and `$app/env/public` agree; `src/env.ts` validates it
	const site = loadEnv(mode, process.cwd(), 'PUBLIC_CORD_SITE').PUBLIC_CORD_SITE === 'true';
	return {
		plugins: [
			sveltekit({
				preprocess: [svelte_preprocess_mdz(), svelte_preprocess_fuz_code(), vitePreprocess()],
				compilerOptions: { runes: true },
				inspector: true,
				// an instance's `spa.html` serves the routes that opt out of prerendering (`/c/[cord_id]`
				// and its tabs); the web server falls back to it for any path with no prerendered file
				adapter: site
					? create_site_adapter()
					: adapter({ pages: BUILD_DIR, assets: BUILD_DIR, fallback: 'spa.html' }),
				// the site prerenders only its own pages, and the `reroute` hook 404s the
				// rest, so an instance's pages go unseen; a site page going unseen still fails
				...(site && {
					prerender: {
						entries: [...CORD_SITE_PATHS],
						handleUnseenRoutes: ({ routes, message }) => {
							if (routes.some(is_cord_site_path)) throw Error(message);
						}
					}
				}),
				paths: { relative: false }, // use root-absolute paths for SSR path comparison: https://svelte.dev/docs/kit/configuration#paths
				version: { name: execSync('git rev-parse HEAD').toString().trim() }

				// Example CSP using https://ui.fuz.dev/docs/csp
				//
				// import {create_csp_directives} from '@fuzdev/fuz_ui/csp.ts';
				// import {csp_directives_of_fuzdev} from '@fuzdev/fuz_ui/csp_of_fuzdev.ts';
				// csp: {
				// 	directives: create_csp_directives({
				// 		extend: [csp_directives_of_fuzdev],
				// 	}),
				// },
			}),
			vite_plugin_fuz_css(),
			vite_plugin_pkg_json()
		],
		test: {
			projects: [
				{
					extends: true,
					test: {
						name: 'unit',
						include: ['src/test/**/*.test.ts'],
						exclude: ['src/test/**/*.cross.test.ts'],
						setupFiles: ['./src/test/logged_error_gate.ts'],
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
	};
});
