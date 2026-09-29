import { readFileSync } from 'node:fs';
import { assert, describe, test } from 'vitest';

import {
	CORD_ADMIN,
	CORD_MEMBER,
	CORD_OWNER,
	type CordRole,
	cord_leaves_no_owner,
	cord_role_can_manage,
	cord_role_can_remove,
	cord_role_can_set_member_roles
} from '#lib/cord_roles.ts';

const ROLES: Array<CordRole> = [CORD_OWNER, CORD_ADMIN, CORD_MEMBER];

describe('the member rules', () => {
	test('only owners set member roles', () => {
		assert.deepEqual(
			ROLES.map((role) => cord_role_can_set_member_roles(role)),
			[true, false, false]
		);
	});

	test('owners and admins manage the cord', () => {
		assert.deepEqual(
			ROLES.map((role) => cord_role_can_manage(role)),
			[true, true, false]
		);
	});

	test('removal follows the authority order', () => {
		// rows are the remover, columns the target: owner, admin, member
		const table: Record<CordRole, [boolean, boolean, boolean]> = {
			cord_owner: [true, true, true],
			cord_admin: [false, false, true],
			cord_member: [false, false, false]
		};
		for (const role of ROLES) {
			assert.deepEqual(
				ROLES.map((target) => cord_role_can_remove(role, target)),
				table[role],
				role
			);
		}
	});

	test('the last owner stays', () => {
		for (const owner_count of [0, 1, 2, 3]) {
			for (const current of ROLES) {
				for (const next of [...ROLES, null]) {
					const expected = current === CORD_OWNER && next !== CORD_OWNER && owner_count <= 1;
					assert.strictEqual(
						cord_leaves_no_owner(owner_count, current, next),
						expected,
						JSON.stringify({ owner_count, current, next })
					);
				}
			}
		}
		// the cases the server's tests name
		assert.isTrue(cord_leaves_no_owner(1, CORD_OWNER, null), 'the only owner leaving');
		assert.isTrue(cord_leaves_no_owner(1, CORD_OWNER, CORD_ADMIN), 'the only owner stepping down');
		assert.isFalse(cord_leaves_no_owner(1, CORD_OWNER, CORD_OWNER), 'keeping the role');
		assert.isFalse(cord_leaves_no_owner(2, CORD_OWNER, null), 'another owner remains');
		assert.isTrue(cord_leaves_no_owner(0, CORD_OWNER, null), 'a count of zero still refuses');
	});

	test('agree with every case in the server’s own tests', () => {
		// replays every call of the rules in `cord_authz.rs`'s tests, expanding
		// `for x in [..]` loops over roles, so a change to the server's rules and
		// its tests fails here until the twin follows. Any call the patterns
		// can't read (another assert form, a new loop shape) fails the count
		const source = readFileSync('crates/cord_server/src/cord_authz.rs', 'utf8');
		const tests = source.slice(source.indexOf('#[cfg(test)]'));
		const RUST_ROLES: Record<string, CordRole> = {
			Owner: CORD_OWNER,
			Admin: CORD_ADMIN,
			Member: CORD_MEMBER
		};

		// each call, with the loop variables in scope where it sits
		const calls: Array<{ line: string; bindings: Map<string, Array<CordRole>> }> = [];
		const loops: Array<{ name: string; roles: Array<CordRole>; depth: number }> = [];
		let depth = 0;
		for (const line of tests.split('\n')) {
			const loop = /^\s*for (\w+) in \[([^\]]+)\] \{\s*$/.exec(line);
			if (loop) {
				const roles = loop[2]!.split(',').map((r) => {
					const role = RUST_ROLES[r.trim().replace(/^CordRole::/, '')];
					assert.ok(role, `a loop over something other than roles: ${line}`);
					return role;
				});
				loops.push({ name: loop[1]!, roles, depth });
			}
			if (/can_set_member_roles\(|can_remove\(|leaves_no_owner\(/.test(line)) {
				calls.push({ line, bindings: new Map(loops.map((l) => [l.name, l.roles])) });
			}
			for (const c of line) {
				if (c === '{') depth++;
				else if (c === '}') {
					depth--;
					while (loops.length > 0 && loops.at(-1)!.depth >= depth) loops.pop();
				}
			}
		}

		// every assignment of the loop variables a call sits in
		const expand = (
			names: Array<string>,
			bindings: Map<string, Array<CordRole>>
		): Array<Record<string, CordRole>> =>
			names.reduce<Array<Record<string, CordRole>>>(
				(acc, name) => {
					const roles = bindings.get(name);
					if (!roles) return acc;
					return acc.flatMap((env) => roles.map((role) => ({ ...env, [name]: role })));
				},
				[{}]
			);
		const ROLE = '((?:CordRole::)?\\w+)';
		const to_role = (token: string, env: Record<string, CordRole>, line: string): CordRole => {
			const role = RUST_ROLES[token.replace(/^CordRole::/, '')] ?? env[token];
			assert.ok(role, `not a role or a loop variable over roles: ${token} in ${line}`);
			return role;
		};

		const SET_ROLES = new RegExp(`^\\s*assert!\\((!?)${ROLE}\\.can_set_member_roles\\(\\)[,)]`);
		const REMOVE = new RegExp(`^\\s*assert!\\((!?)${ROLE}\\.can_remove\\(${ROLE}\\)[,)]`);
		const LAST_OWNER = new RegExp(
			`^\\s*assert!\\((!?)leaves_no_owner\\((\\d+), ${ROLE}, (?:None|Some\\(${ROLE}\\))\\)[,)]`
		);

		const replayed = { set_roles: 0, remove: 0, last_owner: 0 };
		for (const { line, bindings } of calls) {
			const envs = expand([...bindings.keys()], bindings);
			let match: RegExpExecArray | null;
			if ((match = SET_ROLES.exec(line))) {
				const [, negated, role] = match;
				for (const env of envs) {
					assert.strictEqual(
						cord_role_can_set_member_roles(to_role(role!, env, line)),
						!negated,
						`${line} ${JSON.stringify(env)}`
					);
				}
				replayed.set_roles++;
			} else if ((match = REMOVE.exec(line))) {
				const [, negated, remover, target] = match;
				for (const env of envs) {
					assert.strictEqual(
						cord_role_can_remove(to_role(remover!, env, line), to_role(target!, env, line)),
						!negated,
						`${line} ${JSON.stringify(env)}`
					);
				}
				replayed.remove++;
			} else if ((match = LAST_OWNER.exec(line))) {
				const [, negated, count, current, next] = match;
				for (const env of envs) {
					assert.strictEqual(
						cord_leaves_no_owner(
							Number(count),
							to_role(current!, env, line),
							next ? to_role(next, env, line) : null
						),
						!negated,
						`${line} ${JSON.stringify(env)}`
					);
				}
				replayed.last_owner++;
			} else {
				assert.fail(`a call of the member rules this test can't replay: ${line.trim()}`);
			}
		}

		// every raw call in the test module was read and replayed, and each rule has some
		const count = (pattern: RegExp): number => tests.match(pattern)?.length ?? 0;
		assert.strictEqual(replayed.set_roles, count(/\.can_set_member_roles\(/g));
		assert.strictEqual(replayed.remove, count(/\.can_remove\(/g));
		assert.strictEqual(replayed.last_owner, count(/\bleaves_no_owner\(/g));
		assert.isAbove(replayed.set_roles, 0);
		assert.isAbove(replayed.remove, 0);
		assert.isAbove(replayed.last_owner, 0);
	});
});
