<script lang="ts">
	import MediaCheck from '#lib/ui/MediaCheck.svelte';
</script>

<svelte:head>
	<title>a/v check - Cord</title>
</svelte:head>

<main class="p_lg">
	<h1>a/v check</h1>
	<p>
		Check your mic, camera, and screen share before a line. It all stays in this browser, and
		nothing is sent anywhere.
	</p>
	<MediaCheck />
</main>
