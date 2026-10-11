<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { untrack } from 'svelte';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import PendingButton from '@fuzdev/fuz_ui/PendingButton.svelte';
	import LoginForm from '@fuzdev/fuz_app/ui/LoginForm.svelte';
	import LogoutButton from '@fuzdev/fuz_app/ui/LogoutButton.svelte';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import {
		CordJoinState,
		cord_join_session_storage,
		to_cord_join_recheck_error,
		to_cord_join_view,
		type CordJoinViewInput
	} from '#lib/ui/cord_join_state.svelte.ts';
	import CordNewcomerForm from '#lib/ui/CordNewcomerForm.svelte';

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();
	const cords_state = cords_state_context.get();

	// the token rides in the fragment (`/join#<token>`), which never reaches a
	// server; this tab's `sessionStorage` keeps it, and a newcomer's
	// redemption, once the fragment is gone, so a reload mid-join still finds them
	const join_state = new CordJoinState({ api, storage: cord_join_session_storage() });

	$effect(() => {
		const hash = page.url.hash;
		untrack(() => {
			join_state.load(hash);
			// drop the token from the address bar and this history entry
			if (hash) void goto(resolve('/join'), { replace: true, reset: false });
		});
	});

	// a redemption belongs to whoever was signing up; `logout()` drops it
	$effect(() => auth_state.on_logout(() => join_state.forget_on_logout()));

	const account_id = $derived(auth_state.signed_in ? (auth_state.account?.id ?? null) : null);
	const account_username = $derived(
		auth_state.signed_in ? (auth_state.account?.username ?? null) : null
	);

	// preview the link for whoever is here: signed out, or once the account has loaded
	$effect(() => {
		if (!auth_state.session_checked || !join_state.token_loaded) return;
		if (auth_state.verified && !auth_state.signed_in) return;
		void join_state.fetch_preview(account_id);
	});

	const view_input: CordJoinViewInput = $derived({
		token_loaded: join_state.token_loaded,
		token: join_state.token,
		session_checked: auth_state.session_checked,
		verified: auth_state.verified,
		signed_in: auth_state.signed_in,
		account_id,
		account_username,
		preview_for: join_state.preview_for,
		preview_status: join_state.preview.status,
		preview: join_state.preview.data,
		preview_error_data: join_state.preview.error_data,
		dead: join_state.dead,
		newcomer_running: join_state.newcomer_running,
		redeemed: join_state.redeemed,
		newcomers_closed: join_state.newcomers_closed
	});
	const view = $derived(to_cord_join_view(view_input));
	// a failed re-preview keeps the last view (and a form's input) on screen
	const recheck_error = $derived(to_cord_join_recheck_error(view_input));

	// a link that's done its job, or can't do one, needn't linger in storage,
	// unless it holds a newcomer's redemption and someone else is signed in
	$effect(() => {
		if (view.kind === 'dead' || view.kind === 'member') {
			join_state.forget(auth_state.signed_in, account_username);
		}
	});

	// someone new chooses between picking a username and signing in
	let has_account = $state.raw(false);

	const choose_has_account = (value: boolean): void => {
		if (join_state.newcomer_running) return;
		has_account = value;
	};

	const go_to_cord = async (cord_id: string): Promise<void> => {
		join_state.forget(auth_state.signed_in, account_username);
		// reload rather than add the cord: a newcomer's first load of the list
		// can be in flight from before the accept, and this one supersedes it
		void cords_state.fetch();
		await goto(resolve('/c/[cord_id]', { cord_id }));
	};

	const join = async (): Promise<void> => {
		if (join_state.accept.loading) return;
		const cord = await join_state.accept_invite();
		if (cord) await go_to_cord(cord.id);
	};

	const join_as_newcomer = async (username: string, password: string, password_confirm: string) => {
		const outcome = await join_state.join_as_newcomer(
			username,
			password,
			password_confirm,
			(u, p) => auth_state.signup(u, p)
		);
		if (outcome.kind === 'joined') await go_to_cord(outcome.cord.id);
		return outcome;
	};
</script>

<svelte:head>
	<title>join - Cord</title>
</svelte:head>

<main class="width_atmost_sm mx_auto p_lg">
	{#if view.kind === 'checking'}
		<p class="text_50">checking your link...</p>
	{:else if view.kind === 'dead'}
		<h1>this link doesn't work anymore</h1>
		<p>Ask whoever sent it for a new one.</p>
	{:else if view.kind === 'error'}
		<h1>join a cord</h1>
		<Alert status="error"><p>Couldn't check this link: {view.message}</p></Alert>
		<button type="button" onclick={() => void join_state.fetch_preview(account_id)}>
			try again
		</button>
	{:else if view.kind === 'member'}
		<h1>{view.cord.name}</h1>
		<p>You're already in this cord.</p>
		<p><a href={resolve('/c/[cord_id]', { cord_id: view.cord.id })}>Go to {view.cord.name}</a></p>
	{:else if view.kind === 'join'}
		<h1>join {view.cord.name}</h1>
		<p>
			You're invited to <strong>{view.cord.name}</strong>. You'll join as
			<strong>{auth_state.account?.username}</strong>.
		</p>
		<PendingButton
			pending={join_state.accept.loading}
			disabled={false}
			aria-disabled={join_state.accept.loading}
			onclick={join}
		>
			join
		</PendingButton>
		{#if join_state.accept.error}
			<p class="color_c_50 font_size_sm">Couldn't join: {join_state.accept.error}</p>
		{/if}
		<p>Not you? <LogoutButton /></p>
	{:else if view.kind === 'newcomer'}
		<h1>join {view.cord.name}</h1>
		<p>You're invited to <strong>{view.cord.name}</strong>, a cord on this Cord instance.</p>
		<div class="row gap_sm" role="group" aria-label="do you have an account here?">
			<button
				type="button"
				class:selected={!has_account}
				aria-pressed={!has_account}
				aria-disabled={join_state.newcomer_running}
				onclick={() => choose_has_account(false)}
			>
				I'm new here
			</button>
			<button
				type="button"
				class:selected={has_account}
				aria-pressed={has_account}
				aria-disabled={join_state.newcomer_running}
				onclick={() => choose_has_account(true)}
			>
				I already have an account
			</button>
		</div>
		{#if has_account}
			<p>Log in, and then you can join.</p>
			<LoginForm username_label="username" redirect_on_login={resolve('/join')} />
		{:else}
			{#if join_state.redeemed}
				<p>Your username is reserved. Pick a password to finish your account on this instance.</p>
			{:else}
				<p>Pick a username and a password. They're your account on this instance.</p>
			{/if}
			<CordNewcomerForm
				onsubmit={join_as_newcomer}
				pending={join_state.newcomer_running}
				signup_error={auth_state.verify_error}
				reserved_username={join_state.redeemed?.username ?? null}
			/>
		{/if}
	{:else}
		<h1>sign in to join</h1>
		{#if view.newcomers_closed}
			<p>
				This link can't bring new people onto this instance anymore, so it can't make you an
				account. If you already have an account here, you can still log in and join with it.
			</p>
		{:else}
			<p>
				This invite is for people who already have an account here. Log in and you'll see which cord
				it's for.
			</p>
		{/if}
		<LoginForm username_label="username" redirect_on_login={resolve('/join')} />
	{/if}
	{#if recheck_error !== null && view.kind !== 'dead' && view.kind !== 'checking'}
		<p class="text_50 font_size_sm" title={recheck_error}>Couldn't re-check this link just now.</p>
	{/if}
</main>
