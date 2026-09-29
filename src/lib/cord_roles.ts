/**
 * Cord's membership roles, the twin of `crates/cord_server/src/roles.rs`.
 *
 * Each is a spine role grant scoped to one cord (`scope_kind: 'cord'`,
 * `scope_id` the cord's id), and an account holds at most one per cord. Only
 * Cord's own actions confer them; the admin role UI and role-grant offers
 * can't.
 *
 * @module
 */

import { z } from 'zod';

/** The cord's owner. */
export const CORD_OWNER = 'cord_owner';

/** A cord admin, managing the cord under its owner. */
export const CORD_ADMIN = 'cord_admin';

/** A cord member. */
export const CORD_MEMBER = 'cord_member';

/** A role in one cord, highest authority first. */
export const CordRole = z.enum([CORD_OWNER, CORD_ADMIN, CORD_MEMBER]);
export type CordRole = z.infer<typeof CordRole>;

/** Human labels for the cord roles. */
export const CORD_ROLE_LABELS: Record<CordRole, string> = {
	cord_owner: 'owner',
	cord_admin: 'admin',
	cord_member: 'member'
};

/**
 * Whether a role may manage its cord (rename it, and later manage its members
 * and spaces).
 */
export const cord_role_can_manage = (role: CordRole): boolean =>
	role === CORD_OWNER || role === CORD_ADMIN;
