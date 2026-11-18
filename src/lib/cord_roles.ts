/**
 * Cord's membership roles and member rules, the twin of
 * `crates/cord_server/src/roles.rs` (the roles) and
 * `crates/cord_server/src/cord_authz.rs` (the member rules, which the cord
 * page uses to decide which controls to offer; the server applies them
 * again).
 *
 * Each role is a spine role grant scoped to one cord (`scope_kind: 'cord'`,
 * `scope_id` the cord's id), and an actor holds at most one per cord. Only
 * Cord's own actions confer them; the admin role UI and role-grant offers
 * can't.
 *
 * @module
 */

import { one_of, type Infer } from '@fuzdev/fuz_schema/schema.ts';

/** The cord's owner. */
export const CORD_OWNER = 'cord_owner';

/** A cord admin, managing the cord under its owner. */
export const CORD_ADMIN = 'cord_admin';

/** A cord member. */
export const CORD_MEMBER = 'cord_member';

/** A role in one cord, highest authority first. */
export const CordRole = one_of([CORD_OWNER, CORD_ADMIN, CORD_MEMBER]);
export type CordRole = Infer<typeof CordRole>;

/** Human labels for the cord roles. */
export const CORD_ROLE_LABELS: Record<CordRole, string> = {
	cord_owner: 'owner',
	cord_admin: 'admin',
	cord_member: 'member'
};

/**
 * Whether a role may manage its cord (rename it, manage its invite links
 * and its spaces, remove plain members).
 */
export const cord_role_can_manage = (role: CordRole): boolean =>
	role === CORD_OWNER || role === CORD_ADMIN;

/**
 * Whether a role may set members' roles (to owner, admin, or member): owners
 * only. The twin of the server's `CordRole::can_set_member_roles`.
 */
export const cord_role_can_set_member_roles = (role: CordRole): boolean => role === CORD_OWNER;

/**
 * Whether `role` may remove a member holding `target`: an owner removes
 * anyone, an admin removes plain members only, and a member removes no one.
 * The last-owner rule (`cord_leaves_no_owner`) applies on top, and nobody
 * removes themself; they leave instead. The twin of the server's
 * `CordRole::can_remove`.
 */
export const cord_role_can_remove = (role: CordRole, target: CordRole): boolean => {
	switch (role) {
		case CORD_OWNER:
			return true;
		case CORD_ADMIN:
			return target === CORD_MEMBER;
		case CORD_MEMBER:
			return false;
	}
};

/**
 * Whether moving one member from `current` to `next` (`null` when they leave
 * or are removed) would leave the cord with no owner, given `owner_count`
 * owners before the change. A cord always keeps an owner: the last one can't
 * leave, be removed, or step down. The twin of the server's `leaves_no_owner`.
 */
export const cord_leaves_no_owner = (
	owner_count: number,
	current: CordRole,
	next: CordRole | null
): boolean => current === CORD_OWNER && next !== CORD_OWNER && owner_count <= 1;
