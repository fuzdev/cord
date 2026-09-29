<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import { Uuid } from '@fuzdev/fuz_util/id.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { CordState } from '#lib/ui/cords_state.svelte.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';
	import { CORD_ROLE_LABELS, cord_role_can_manage } from '#lib/cord_roles.ts';
	import { ERROR_CORD_NOT_FOUND } from '#lib/cord_action_specs.ts';

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	const cord_state = new CordState({ api });

	// a malformed id can't name a cord, so it reads as not found without a request
	const cord_id = $derived(Uuid.safeParse(page.params.cord_id).data);
	const account_id = $derived(auth_state.signed_in ? auth_state.account?.id : undefined);
	$effect(() => {
		if (account_id && cord_id) void cord_state.fetch(cord_id);
	});

	const cord = $derived(cord_state.cord);
	// the server answers a cord you aren't in exactly as one that doesn't exist
	const not_found = $derived(
		!cord_id ||
			(cord_state.load.failed &&
				(cord_state.load.error_data as { data?: { reason?: unknown } } | null)?.data?.reason ===
					ERROR_CORD_NOT_FOUND)
	);
</script>

<svelte:head>
	<title>{cord ? `${cord.name} - Cord` : 'Cord'}</title>
</svelte:head>

<main class="width_atmost_sm mx_auto p_lg">
	{#if !auth_state.session_checked}
		<p class="text_50">checking session...</p>
	{:else if !auth_state.signed_in}
		<p>You're not signed in. <a href={resolve('/login')}>Log in</a></p>
	{:else if not_found}
		<h1>cord not found</h1>
		<p>There's no cord here, or you're not in it.</p>
	{:else if cord_state.load.failed}
		<Alert status="error"><p>Couldn't load the cord: {cord_state.load.error}</p></Alert>
	{:else if !cord}
		<p class="text_50">loading cord...</p>
	{:else}
		<h1>{cord.name}</h1>
		<p>You're its <strong>{CORD_ROLE_LABELS[cord.role]}</strong>.</p>
		<section>
			<h2>spaces</h2>
			<p class="text_50">
				Spaces will live here: live rooms, chats, forums, and feeds. None exist yet.
			</p>
		</section>
		{#if cord_role_can_manage(cord.role)}
			<section>
				<h2>rename</h2>
				{#key cord.id}
					<CordNameForm
						onsubmit={(name) => cord_state.rename_cord(name)}
						submit_label="rename"
						initial_name={cord.name}
						pending={cord_state.rename.loading}
						error={cord_state.rename.error}
					/>
				{/key}
			</section>
		{/if}
	{/if}
	<p><a href={resolve('/')}>home</a></p>
</main>
