<script lang="ts">
	import { cord_page_state_context } from '#lib/ui/cord_page_state.svelte.ts';
	import CordNameForm from '#lib/ui/CordNameForm.svelte';

	// the layout mounts this page only for a loaded cord and the viewer's role
	// in it, but anyone in the cord can open its URL, so it guards itself
	const cord_page = cord_page_state_context.get();
	const { cord_state } = cord_page;
</script>

<svelte:head>
	<title>{cord_page.cord ? `settings - ${cord_page.cord.name} - Cord` : 'settings - Cord'}</title>
</svelte:head>

{#if !cord_page.can_manage}
	<p class="text_50">Only this cord's owners and admins change its name.</p>
{:else if cord_page.cord}
	<section>
		<h2>rename</h2>
		{#key cord_page.cord.id}
			<CordNameForm
				onsubmit={(name) => cord_state.rename_cord(name)}
				submit_label="rename"
				initial_name={cord_page.cord.name}
				pending={cord_state.rename.loading}
				error={cord_state.rename.error}
			/>
		{/key}
	</section>
{/if}
