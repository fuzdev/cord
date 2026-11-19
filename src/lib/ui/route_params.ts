/**
 * Reading a cord route's params.
 *
 * @module
 */

import { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';
import { parse } from '@fuzdev/fuz_schema/schema.ts';

/**
 * A route param as a `Uuid`, `undefined` when it isn't one: a malformed id
 * names nothing, so a page reads it as not found without a request. `Uuid`
 * lowercases, the way the server answers ids.
 */
export const to_uuid_param = (param: string | undefined): Uuid | undefined => {
	const parsed = parse(Uuid, param);
	return parsed.ok ? parsed.value : undefined;
};
