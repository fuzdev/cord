<script lang="ts">
	/**
	 * The form for minting a cord's invite link: an expiry (7 days to start,
	 * or never), an optional use limit, and, for instance admins, whether the
	 * link also lets someone new join the instance, which needs both an expiry
	 * of at most 30 days and a use limit.
	 *
	 * @module
	 */

	import PendingButton from '@fuzdev/fuz_ui/PendingButton.svelte';
	import type { Uuid } from '@fuzdev/fuz_util/id.ts';

	import { CORD_INVITE_MAX_USES_MAX } from '../cord_action_specs.ts';
	import {
		CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT,
		cord_invite_expiry_choices,
		to_cord_invite_create_request,
		type CordInviteCreateRequest
	} from './cord_invites_state.svelte.ts';

	const {
		cord_id,
		can_admit_newcomers,
		onsubmit,
		pending = false,
		error = null
	}: {
		cord_id: Uuid;
		/** Whether to offer the newcomer toggle: the viewer is an instance admin. */
		can_admit_newcomers: boolean;
		/** Called with a checked request; resolves to whether minting succeeded. */
		onsubmit: (request: CordInviteCreateRequest) => Promise<boolean>;
		/** True while the mint request runs. */
		pending?: boolean;
		/** The server's error for the last mint, if any. */
		error?: string | null;
	} = $props();

	let expires_in_seconds: number | null = $state.raw(CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT);
	let max_uses: number | null | undefined = $state.raw(null);
	let admits_newcomers = $state.raw(false);
	let invalid: string | null = $state.raw(null);

	const choices = $derived(cord_invite_expiry_choices(admits_newcomers));

	const toggle_newcomers = (on: boolean): void => {
		admits_newcomers = on;
		// a newcomer link can't keep an expiry it doesn't allow
		if (on && !choices.some((c) => c.seconds === expires_in_seconds)) {
			expires_in_seconds = CORD_INVITE_EXPIRES_IN_SECONDS_DEFAULT;
		}
	};

	const submit = async (): Promise<void> => {
		if (pending) return;
		const checked = to_cord_invite_create_request(cord_id, {
			expires_in_seconds,
			max_uses,
			admits_newcomers
		});
		if (!checked.ok) {
			invalid = checked.message;
			return;
		}
		invalid = null;
		await onsubmit(checked.value);
	};
</script>

<form
	class="width_atmost_md"
	oninput={() => {
		invalid = null;
	}}
	onsubmit={(e) => {
		e.preventDefault();
		void submit();
	}}
>
	{#if can_admit_newcomers}
		<label class="row gap_sm">
			<input
				type="checkbox"
				name="admits_newcomers"
				checked={admits_newcomers}
				onchange={(e) => toggle_newcomers(e.currentTarget.checked)}
			/>
			<span>also lets someone new join the instance</span>
		</label>
		{#if admits_newcomers}
			<p class="font_size_sm">
				Someone without an account here picks a username and password with this link, then joins the
				cord. It needs an expiry of up to 30 days and a use limit; keep the limit to the people you
				have in mind.
			</p>
		{/if}
	{/if}
	<label>
		<div class="title">expires</div>
		<select
			name="expires_in_seconds"
			value={String(expires_in_seconds)}
			onchange={(e) => {
				const v = e.currentTarget.value;
				expires_in_seconds = v === 'null' ? null : Number(v);
			}}
		>
			{#each choices as choice (choice.label)}
				<option value={String(choice.seconds)}>
					{choice.seconds === null ? 'never expires' : `in ${choice.label}`}
				</option>
			{/each}
		</select>
	</label>
	<label>
		<div class="title">use limit{admits_newcomers ? '' : ' (optional)'}</div>
		<input
			name="max_uses"
			type="number"
			min="1"
			max={CORD_INVITE_MAX_USES_MAX}
			step="1"
			bind:value={max_uses}
			placeholder={admits_newcomers ? 'how many people' : 'no limit'}
			readonly={pending}
		/>
	</label>
	<div class="row gap_sm">
		<PendingButton {pending} disabled={false} aria-disabled={pending} onclick={submit}>
			make a link
		</PendingButton>
	</div>
	{#if invalid ?? error}
		<p class="color_c_50 font_size_sm mt_xs mb_0">{invalid ?? error}</p>
	{/if}
</form>
