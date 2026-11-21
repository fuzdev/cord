import type { Reroute } from '@sveltejs/kit/hooks';
import { PUBLIC_CORD_SITE } from '$app/env/public';

import { CORD_SITE_NOT_FOUND_PATH, is_cord_site_path } from '#lib/cord_site.ts';

// on the static site, an instance's pages (log in, admin, a cord) are a 404
// rather than a page that would call a server that isn't there; this runs on
// the client too, so the fallback page can't render them either
export const reroute: Reroute = ({ url }) => {
	if (PUBLIC_CORD_SITE && !is_cord_site_path(url.pathname)) return CORD_SITE_NOT_FOUND_PATH;
	return undefined;
};
