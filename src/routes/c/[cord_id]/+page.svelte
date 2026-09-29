<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';
	import { Uuid } from '@fuzdev/fuz_util/id.ts';

	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { CordState } from '#lib/ui/cords_state.svelte.ts';
	import { CordMembersState } from '#lib/ui/cord_members_state.svelte.ts';
	import { CordInvitesState } from '#lib/ui/cord_invites_state.svelte.ts';
	import { rpc_error_reason } from '#lib/ui/cord_errors.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';
	import CordMembers from '#lib/ui/CordMembers.svelte';
	import CordInvites from '#lib/ui/CordInvites.svelte';
	import { CORD_ROLE_LABELS, cord_role_can_manage } from '#lib/cord_roles.ts';
	import { ERROR_CORD_NOT_FOUND } from '#lib/cord_action_specs.ts';

	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();

	const cord_state = new CordState({ api });
	const members_state = new CordMembersState({ api });
	const invites_state = new CordInvitesState({ api });

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
				rpc_error_reason(cord_state.load.error_data) === ERROR_CORD_NOT_FOUND)
	);

	// whether the member list is this cord's, loaded; a list still loading,
	// failed, or left from another cord says nothing about this one
	const members_loaded = $derived(!!cord && members_state.loaded_cord_id === cord.id);

	// the viewer's role, from this cord's member list once it's loaded (so
	// their own role change shows at once), else from the cord
	const self_actor_id = $derived(auth_state.actor?.id ?? null);
	const role = $derived(
		(members_loaded ? members_state.get_member(self_actor_id)?.role : undefined) ?? cord?.role
	);
	const can_manage = $derived(role !== undefined && cord_role_can_manage(role));

	// `fetch` reads nothing trackably, so these rerun only when their inputs change
	const loaded_cord_id = $derived(cord?.id);
	$effect(() => {
		if (loaded_cord_id) void members_state.fetch(loaded_cord_id);
	});
	$effect(() => {
		if (loaded_cord_id && can_manage) void invites_state.fetch(loaded_cord_id);
	});

	const left = async (): Promise<void> => {
		await goto(resolve('/'));
	};
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
	{:else if role}
		<h1>{cord.name}</h1>
		<p>You're its <strong>{CORD_ROLE_LABELS[role]}</strong>.</p>
		<section>
			<h2>spaces</h2>
			<p class="text_50">
				Spaces will live here: live rooms, chats, forums, and feeds. None exist yet.
			</p>
		</section>
		<CordMembers {members_state} {role} {self_actor_id} onleft={left} />
		{#if can_manage}
			<CordInvites
				{invites_state}
				cord_id={cord.id}
				members={members_loaded ? members_state.members : null}
				can_admit_newcomers={auth_state.is_instance_admin}
			/>
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
