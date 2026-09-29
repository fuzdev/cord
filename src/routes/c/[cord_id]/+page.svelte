<script lang="ts">
	import { CORD_ROLE_LABELS } from '#lib/cord_roles.ts';
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';

	// the layout mounts this page only for a loaded cord and the viewer's role in it
	const cord_page = cord_page_state_context.get();
</script>

<svelte:head>
	<title>{cord_page.cord ? `${cord_page.cord.name} - Cord` : 'Cord'}</title>
</svelte:head>

{#if cord_page.role}
	<p>You're its <strong>{CORD_ROLE_LABELS[cord_page.role]}</strong>.</p>
{/if}
<section>
	<h2>spaces</h2>
	<p class="text_50">
		Spaces will live here: live rooms, chats, forums, and feeds. None exist yet.
	</p>
</section>
