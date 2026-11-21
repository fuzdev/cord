/**
 * cord.fuz.dev, the static site: this repo's frontend built with
 * `PUBLIC_CORD_SITE=true` and served by GitHub Pages, with no server behind
 * it. The site build prerenders only `CORD_SITE_PATHS`, the `reroute` hook
 * sends every other path to a 404, and the root layout never checks a
 * session, so nothing asks for an `/api` that isn't there.
 *
 * Plain data, so `vite.config.ts` imports it as well as the app.
 *
 * @module
 */

/** The pages the site serves; everything else is an instance's. */
export const CORD_SITE_PATHS = ['/', '/about', '/av'] as const;

/** The site's domain, written to the build's `CNAME` for GitHub Pages. */
export const CORD_SITE_DOMAIN = 'cord.fuz.dev';

/** Matches no route, so a path rerouted to it renders the 404 page. */
export const CORD_SITE_NOT_FOUND_PATH = '/__not_on_the_site';

export const is_cord_site_path = (pathname: string): boolean =>
	(CORD_SITE_PATHS as ReadonlyArray<string>).includes(pathname);
