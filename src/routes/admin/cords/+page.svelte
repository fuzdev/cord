<script lang="ts">
	import { resolve } from '$app/paths';
	import Alert from '@fuzdev/fuz_ui/Alert.svelte';

	import { cord_rpc_context } from '#lib/ui/rpc.ts';
	import { AdminCordsState } from '#lib/ui/cords_state.svelte.ts';

	const { api } = cord_rpc_context.get();

	// the admin layout renders this only for a signed-in admin
	const admin_cords_state = new AdminCordsState({ api });
	void admin_cords_state.fetch();
</script>

<svelte:head>
	<title>cords - admin - Cord</title>
</svelte:head>

<div class="p_lg">
	<h1>cords</h1>
	<p>
		Every cord on this instance, for operations. Cords you aren't in stay out of your everyday view,
		but as this instance's operator you can read everything in its database.
	</p>
	{#if admin_cords_state.list.failed}
		<Alert status="error"><p>Couldn't load the cords: {admin_cords_state.list.error}</p></Alert>
	{:else if !admin_cords_state.list.succeeded}
		<p class="text_50">loading cords...</p>
	{:else if admin_cords_state.cords.length === 0}
		<p>No cords yet. Create the first one from the <a href={resolve('/')}>home page</a>.</p>
	{:else}
		<table>
			<thead>
				<tr>
					<th>name</th>
					<th>members</th>
					<th>created</th>
				</tr>
			</thead>
			<tbody>
				{#each admin_cords_state.cords as cord (cord.id)}
					<tr>
						<td>{cord.name}</td>
						<td>{cord.member_count}</td>
						<td><time datetime={cord.created_at}>{cord.created_at.slice(0, 10)}</time></td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</div>
