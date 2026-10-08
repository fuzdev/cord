<script lang="ts">
	/**
	 * A cord's members, for anyone in it: each member's name and role, role
	 * controls for owners, remove for owners and admins where the member rules
	 * allow it, and leaving, with a confirm step. The rules come from the
	 * twins in `cord_roles.ts`; the server applies them again.
	 *
	 * A role change is a choice in the select, then "apply": the select alone
	 * changes nothing, since arrowing through a closed select fires `change`
	 * on each step in some browsers. Making someone an owner confirms first
	 * (an owner can change anyone's role and remove anyone, you included), as
	 * does an owner stepping themself down (only another owner can undo it).
	 * A confirm prompt takes focus (on its safe choice) while open, and gives
	 * it back after: to the apply button on cancel, and on confirm to the apply
	 * button while the change runs, then to the select once applied. A
	 * member's controls stay pending until the list reloads with the change.
	 * Removing a member moves focus to the next member's first control, or to
	 * the heading when none follows.
	 *
	 * @module
	 */

	import { tick } from 'svelte';
	import type { Attachment } from 'svelte/attachments';
	import { SvelteMap } from 'svelte/reactivity';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import PendingButton from '@fuzdev/fuz_ui/PendingButton.svelte';
	import { autofocus } from '@fuzdev/fuz_ui/autofocus.svelte.ts';
	import ConfirmButton from '@fuzdev/fuz_app/ui/ConfirmButton.svelte';
	import type { Uuid } from '@fuzdev/fuz_util/id.ts';

	import type { CordMemberJson } from '../cord_action_specs.ts';
	import {
		CORD_OWNER,
		CORD_ROLE_LABELS,
		CordRole,
		cord_leaves_no_owner,
		cord_role_can_remove,
		cord_role_can_set_member_roles
	} from '../cord_roles.ts';
	import type { CordMembersState } from './cord_members_state.svelte.ts';

	const {
		members_state,
		role,
		self_actor_id,
		onleft
	}: {
		members_state: CordMembersState;
		/** The viewer's role in the cord. */
		role: CordRole;
		/** The viewer's actor id, to mark and exempt their own row. */
		self_actor_id: Uuid | null;
		/** Called once the viewer has left the cord. */
		onleft: () => void;
	} = $props();

	const can_set_roles = $derived(cord_role_can_set_member_roles(role));
	const owner_count = $derived(members_state.owner_count);
	const last_owner = $derived(cord_leaves_no_owner(owner_count, role, null));

	// the role chosen in each member's select, until it's applied
	const drafts: SvelteMap<Uuid, CordRole> = new SvelteMap();

	// the change waiting on a confirm, if any
	let confirming: { actor_id: Uuid; next: CordRole; kind: 'step_down' | 'promote' } | null =
		$state.raw(null);

	// focus targets by actor id, for after a change or a confirm
	const select_els: Map<Uuid, HTMLElement> = new Map();
	const apply_els: Map<Uuid, HTMLElement> = new Map();
	const row_els: Map<Uuid, HTMLElement> = new Map();
	let heading_el: HTMLElement | undefined = $state.raw();

	/** An attachment keeping `el` in `els` under `actor_id` while it's mounted. */
	const ref =
		(els: Map<Uuid, HTMLElement>, actor_id: Uuid): Attachment<HTMLElement> =>
		(el) => {
			els.set(actor_id, el);
			return () => {
				if (els.get(actor_id) === el) els.delete(actor_id);
			};
		};

	const focus_el = (el: HTMLElement | undefined): void => {
		// `focusVisible` shows the focus ring on this programmatic move
		el?.focus({ focusVisible: true } as FocusOptions);
	};

	const name_of = (member: CordMemberJson): string => member.display_name ?? member.username;

	const draft_of = (member: CordMemberJson): CordRole => drafts.get(member.actor_id) ?? member.role;

	const apply = (member: CordMemberJson): void => {
		if (members_state.role_changing(member.actor_id)) return;
		const next = draft_of(member);
		if (next === member.role) return;
		if (member.actor_id === self_actor_id) {
			confirming = { actor_id: member.actor_id, next, kind: 'step_down' };
		} else if (next === CORD_OWNER) {
			confirming = { actor_id: member.actor_id, next, kind: 'promote' };
		} else {
			void set_role(member.actor_id, next);
		}
	};

	const set_role = async (actor_id: Uuid, next: CordRole): Promise<void> => {
		await members_state.set_role(actor_id, next);
		// applied or refused, the select shows the member's role as it now stands
		if (drafts.get(actor_id) === next) drafts.delete(actor_id);
		await tick();
		// the reload may move the row, or (stepping down) take the select away
		focus_el(select_els.get(actor_id) ?? row_els.get(actor_id));
	};

	const confirm = async (): Promise<void> => {
		const c = confirming;
		if (!c) return;
		// the prompt's buttons unmount with it, so focus waits on the apply
		// button, which shows the change pending
		focus_el(apply_els.get(c.actor_id));
		confirming = null;
		await set_role(c.actor_id, c.next);
	};

	const cancel = async (): Promise<void> => {
		const c = confirming;
		confirming = null;
		if (!c) return;
		drafts.delete(c.actor_id);
		await tick();
		focus_el(apply_els.get(c.actor_id));
	};

	const cancel_on_escape = (e: KeyboardEvent): void => {
		if (e.key === 'Escape') void cancel();
	};

	const remove = async (actor_id: Uuid): Promise<void> => {
		const members = members_state.members;
		const next = members[members.findIndex((m) => m.actor_id === actor_id) + 1];
		if (!(await members_state.remove_member(actor_id))) return;
		await tick();
		// the removed row took focus with it; unless it has moved on since
		if (document.activeElement && document.activeElement !== document.body) return;
		const row = next && row_els.get(next.actor_id);
		focus_el(row?.querySelector<HTMLElement>('select, button') ?? row ?? heading_el);
	};

	const leave = async (): Promise<void> => {
		if (await members_state.leave_cord()) onleft();
	};
</script>

<section>
	<h2 tabindex="-1" bind:this={heading_el}>members</h2>
	{#if members_state.list.failed && members_state.members.length === 0}
		<Alert status="error"><p>Couldn't load the members: {members_state.list.error}</p></Alert>
	{:else if !members_state.list.succeeded && members_state.members.length === 0}
		<p class="text_50">loading members...</p>
	{:else}
		<ul class="unstyled">
			{#each members_state.members as member (member.actor_id)}
				{@const is_self = member.actor_id === self_actor_id}
				{@const role_error = members_state.role_set.error(member.actor_id)}
				{@const remove_error = members_state.remove.error(member.actor_id)}
				<li class="py_xs" tabindex="-1" {@attach ref(row_els, member.actor_id)}>
					<div class="row gap_sm flex-wrap:wrap">
						<span>
							<strong>{name_of(member)}</strong>
							{#if member.display_name !== null && member.display_name !== member.username}
								<small class="text_50">{member.username}</small>
							{/if}
							{#if is_self}<small class="text_50">(you)</small>{/if}
						</span>
						{#if can_set_roles}
							{@const pending = members_state.role_changing(member.actor_id)}
							{@const draft = draft_of(member)}
							<select
								aria-label="role for {name_of(member)}"
								value={draft}
								aria-disabled={pending}
								{@attach ref(select_els, member.actor_id)}
								onchange={(e) => {
									drafts.set(member.actor_id, e.currentTarget.value as CordRole);
								}}
							>
								{#each CordRole.options as option (option)}
									<option
										value={option}
										disabled={cord_leaves_no_owner(owner_count, member.role, option)}
									>
										{CORD_ROLE_LABELS[option]}
									</option>
								{/each}
							</select>
							<PendingButton
								{pending}
								disabled={false}
								aria-disabled={pending || draft === member.role}
								class="sized_sm"
								title="apply the role for {name_of(member)}"
								onclick={() => apply(member)}
								{@attach ref(apply_els, member.actor_id)}
							>
								apply
							</PendingButton>
						{:else}
							<small class="chip">{CORD_ROLE_LABELS[member.role]}</small>
						{/if}
						{#if !is_self &&
							cord_role_can_remove(role, member.role) &&
							!cord_leaves_no_owner(owner_count, member.role, null)
						}
							<ConfirmButton
								onconfirm={() => void remove(member.actor_id)}
								title="remove {name_of(member)}"
								class="sized_sm"
								label="remove"
								pending={members_state.removing(member.actor_id)}
							>
								{#snippet popover_content(_popover, do_confirm)}
									<button
										type="button"
										class="palette_c background-color:var(--bg_100)"
										onclick={() => do_confirm()}
									>
										<span class="py_sm">remove {name_of(member)} from this cord</span>
									</button>
								{/snippet}
							</ConfirmButton>
						{/if}
					</div>
					{#if confirming?.actor_id === member.actor_id}
						{@const prompt_id = `role-confirm-${member.actor_id}`}
						<!-- focus lands on the safe choice, and the group's label, the question, is read on entry -->
						<div class="row gap_sm flex-wrap:wrap" role="group" aria-labelledby={prompt_id}>
							{#if confirming.kind === 'step_down'}
								<p class="mb_0" id={prompt_id}>
									Step down to {CORD_ROLE_LABELS[confirming.next]}? Only another owner can make you
									an owner again.
								</p>
								<button
									type="button"
									class="sized_sm"
									onclick={confirm}
									onkeydown={cancel_on_escape}
								>
									step down
								</button>
								<button
									type="button"
									class="sized_sm"
									onclick={cancel}
									onkeydown={cancel_on_escape}
									{@attach autofocus()}
								>
									keep owner
								</button>
							{:else}
								<p class="mb_0" id={prompt_id}>
									Make {name_of(member)} an owner? An owner can change anyone's role and remove
									anyone from this cord, you included, and only an owner can undo it.
								</p>
								<button
									type="button"
									class="sized_sm"
									onclick={confirm}
									onkeydown={cancel_on_escape}
								>
									make owner
								</button>
								<button
									type="button"
									class="sized_sm"
									onclick={cancel}
									onkeydown={cancel_on_escape}
									{@attach autofocus()}
								>
									cancel
								</button>
							{/if}
						</div>
					{/if}
					{#if role_error ?? remove_error}
						<p class="color_c_50 font_size_sm mt_0 mb_0">{role_error ?? remove_error}</p>
					{/if}
				</li>
			{/each}
		</ul>
		{#if members_state.list.failed}
			<p class="color_c_50 font_size_sm">
				Couldn't refresh the members: {members_state.list.error}
			</p>
		{/if}

		<h3>leave</h3>
		{#if last_owner}
			<p>
				You're this cord's only owner, so it can't let you go yet. Make someone else an owner first,
				then you can leave.
			</p>
		{:else}
			<p>Leaving takes you out of this cord. To come back, you'll need a new invite.</p>
			<ConfirmButton
				onconfirm={leave}
				title="leave this cord"
				label="leave this cord"
				position="right"
				pending={members_state.leave.loading}
			>
				{#snippet popover_content(_popover, do_confirm)}
					<button
						type="button"
						class="palette_c background-color:var(--bg_100)"
						onclick={() => do_confirm()}
					>
						<span class="py_sm">yes, leave</span>
					</button>
				{/snippet}
			</ConfirmButton>
			{#if members_state.leave.error}
				<p class="color_c_50 font_size_sm">{members_state.leave.error}</p>
			{/if}
		{/if}
	{/if}
</section>
