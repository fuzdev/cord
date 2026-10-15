<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

	import { cords_state_context } from '#lib/ui/cords_state.svelte.ts';
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import CordMembers from '#lib/ui/CordMembers.svelte';

	// the layout mounts this page only for a loaded cord and the viewer's role in it
	const cord_page = cord_page_state_context.get();
	const { members_state } = cord_page;
	const cords_state = cords_state_context.get();

	// opening the tab refreshes the list, so a role changed elsewhere (and the
	// tabs it allows) catches up; a refresh keeps the loaded list showing, and
	// `fetch_members` reads nothing trackably, so this reruns only when the cord changes
	const cord_id = $derived(cord_page.cord?.id);
	$effect(() => {
		if (cord_id) void cord_page.fetch_members(cord_id);
	});

	const left = async (left_cord_id: Uuid): Promise<void> => {
		cords_state.remove_cord(left_cord_id);
		await goto(resolve('/'));
	};
</script>

<svelte:head>
	<title>{cord_page.cord ? `members - ${cord_page.cord.name} - Cord` : 'members - Cord'}</title>
</svelte:head>

{#if cord_id && cord_page.role}
	<!-- until a fetch for this cord starts, the state may still hold another cord's list -->
	{#if members_state.cord_id === cord_id}
		<CordMembers
			{members_state}
			role={cord_page.role}
			self_actor_id={cord_page.self_actor_id}
			onleft={() => left(cord_id)}
		/>
	{:else}
		<p class="text_50">loading members...</p>
	{/if}
{/if}
