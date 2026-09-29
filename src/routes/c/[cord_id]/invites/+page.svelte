<script lang="ts">
	import { cord_auth_state_context } from '#lib/ui/cord_auth_state.svelte.ts';
	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import { CordInvitesState } from '#lib/ui/cord_invites_state.svelte.ts';
	import CordInvites from '#lib/ui/CordInvites.svelte';

	// the layout mounts this page only for a loaded cord and the viewer's role
	// in it, but anyone in the cord can open its URL, so it guards itself
	const auth_state = cord_auth_state_context.get();
	const { api } = cord_rpc_context.get();
	const cord_page = cord_page_state_context.get();

	const invites_state = new CordInvitesState({ api });

	// `fetch` reads nothing trackably, so this reruns only when its inputs change
	const cord_id = $derived(cord_page.cord?.id);
	$effect(() => {
		if (cord_id && cord_page.can_manage) void invites_state.fetch(cord_id);
	});
</script>

<svelte:head>
	<title>{cord_page.cord ? `invites - ${cord_page.cord.name} - Cord` : 'invites - Cord'}</title>
</svelte:head>

{#if !cord_page.can_manage}
	<p class="text_50">Only this cord's owners and admins manage its invite links.</p>
{:else if cord_id}
	<CordInvites
		{invites_state}
		{cord_id}
		members={cord_page.members_loaded ? cord_page.members_state.members : null}
		can_admit_newcomers={auth_state.is_instance_admin}
	/>
{/if}
