import { defineEnvVars } from '@sveltejs/kit/env';

// the frontend's env vars, imported from `$app/env/public`; static, so the
// build bakes them in and the site build drops the instance-only code paths

export const variables = defineEnvVars({
	PUBLIC_CORD_SITE: {
		public: true,
		static: true,
		// strict, so a typo can't build an instance as the site or the reverse
		schema: (value: string | undefined): boolean => {
			if (value === undefined || value === '' || value === 'false') return false;
			if (value === 'true') return true;
			throw Error(`PUBLIC_CORD_SITE must be 'true', 'false', or empty, got '${value}'`);
		},
		description:
			"`true` builds cord.fuz.dev, the static site: no server, only `CORD_SITE_PATHS` (`src/lib/cord_site.ts`); empty builds an instance's frontend"
	}
});
