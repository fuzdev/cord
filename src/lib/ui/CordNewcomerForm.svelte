<script lang="ts">
	/**
	 * The join page's form for someone new to the instance: a username and a
	 * password, checked here first, then handed to `onsubmit`, which runs
	 * redeem, signup, and accept back to back. The form stays mounted with its
	 * input through a failure; while a submit runs its fields are `readonly`
	 * and its button `aria-disabled`, so focus stays put. Once a redemption
	 * has reserved a username, the field shows it read-only: a retry finishes
	 * with that name and never spends another use on a new one.
	 *
	 * @module
	 */

	import PendingButton from '@fuzdev/fuz_ui/PendingButton.svelte';
	import { autofocus } from '@fuzdev/fuz_ui/autofocus.svelte.ts';
	import { FormState } from '@fuzdev/fuz_app/ui/form_state.svelte.ts';
	import { PASSWORD_LENGTH_MIN } from '@fuzdev/fuz_app/auth/password.ts';

	import type { CordNewcomerField, CordNewcomerOutcome } from './cord_join_state.svelte.ts';

	const {
		onsubmit,
		pending = false,
		signup_error = null,
		reserved_username = null
	}: {
		/** Runs the newcomer flow; resolves to how it ended. */
		onsubmit: (
			username: string,
			password: string,
			password_confirm: string
		) => Promise<CordNewcomerOutcome>;
		/** True while the flow runs. */
		pending?: boolean;
		/** Signup's own refusal, when that's where the flow stopped. */
		signup_error?: string | null;
		/** The username a redemption of this link already reserved, which the form keeps read-only. */
		reserved_username?: string | null;
	} = $props();

	const form_state = new FormState<CordNewcomerField>();

	// a reservation restored on mount seeds the field, for if it's dropped later
	// svelte-ignore state_referenced_locally
	let username = $state.raw(reserved_username ?? '');
	const reserved = $derived(reserved_username !== null);
	// the first field to fill on mount; a reservation made later leaves focus be
	// svelte-ignore state_referenced_locally
	const focus_password_on_mount = reserved_username !== null;
	let password = $state.raw('');
	let password_confirm = $state.raw('');

	let field_error: { field: CordNewcomerField; message: string } | null = $state.raw(null);
	let form_error: string | null = $state.raw(null);
	let signup_failed = $state.raw(false);

	const submit = async (): Promise<void> => {
		if (pending) return;
		field_error = null;
		form_error = null;
		signup_failed = false;
		const outcome = await onsubmit(reserved_username ?? username, password, password_confirm);
		switch (outcome.kind) {
			case 'invalid':
				field_error = { field: outcome.field, message: outcome.message };
				form_state.focus(outcome.field);
				break;
			case 'username':
				field_error = { field: 'username', message: outcome.message };
				form_state.focus('username');
				break;
			case 'redeem_failed':
				form_error = outcome.message;
				break;
			case 'signup_failed':
				signup_failed = true;
				break;
			case 'joined':
			case 'dead':
			case 'newcomers_closed':
			case 'accept_failed':
				// the page moves on to another view
				break;
			case 'busy':
				// a submit already running answers for this one
				break;
		}
	};
</script>

<form
	class="width_atmost_md"
	oninput={() => {
		field_error = null;
		form_error = null;
		signup_failed = false;
	}}
	onsubmit={(e) => {
		e.preventDefault();
		void submit();
	}}
	{@attach form_state.form()}
>
	<label>
		<div class="title">{reserved ? 'username, reserved for you' : 'username'}</div>
		<!-- one element either way, so focus survives the switch -->
		<input
			name="username"
			type="text"
			value={reserved_username ?? username}
			oninput={(e) => (username = e.currentTarget.value)}
			placeholder="username"
			autocomplete="username"
			readonly={pending || reserved}
			{@attach focus_password_on_mount ? undefined : autofocus()}
		/>
	</label>
	{#if field_error?.field === 'username'}
		<p class="color_c_50 font_size_sm mt_0 mb_xs">{field_error.message}</p>
	{/if}
	<label>
		<div class="title">password (at least {PASSWORD_LENGTH_MIN} characters)</div>
		<input
			name="password"
			type="password"
			bind:value={password}
			placeholder="password"
			autocomplete="new-password"
			readonly={pending}
			{@attach focus_password_on_mount ? autofocus() : undefined}
		/>
	</label>
	{#if field_error?.field === 'password'}
		<p class="color_c_50 font_size_sm mt_0 mb_xs">{field_error.message}</p>
	{/if}
	<label>
		<div class="title">password again</div>
		<input
			name="password_confirm"
			type="password"
			bind:value={password_confirm}
			placeholder="password again"
			autocomplete="new-password"
			readonly={pending}
		/>
	</label>
	{#if field_error?.field === 'password_confirm'}
		<p class="color_c_50 font_size_sm mt_0 mb_xs">{field_error.message}</p>
	{/if}
	<div class="row gap_sm">
		<PendingButton {pending} disabled={false} aria-disabled={pending} onclick={submit}>
			join
		</PendingButton>
	</div>
	{#if form_error ?? (signup_failed ? signup_error : null)}
		<p class="color_c_50 font_size_sm mt_xs mb_0">
			{form_error ?? signup_error}
		</p>
	{/if}
</form>
