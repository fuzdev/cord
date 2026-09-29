<script lang="ts">
	/**
	 * A route-driven tab bar: a horizontal `<nav>` around `Tab` children,
	 * each of which derives its own selection from the page's path.
	 *
	 * Deliberately not `role="tablist"`: ARIA tabs imply switching state
	 * within a page, and here each tab is a real route, so plain `<nav>` and
	 * `<a>` semantics match the interaction.
	 *
	 * A parallel local copy of visiones' route tabs, pending a shared one in
	 * fuz_app; unlike visiones, cord has no global stylesheet, so the look
	 * lives here and in `Tab.svelte`.
	 *
	 * @module
	 */

	import type { Snippet } from 'svelte';

	const {
		children,
		label
	}: {
		children: Snippet;
		/** The nav's accessible name. */
		label?: string;
	} = $props();
</script>

<nav class="tabs" aria-label={label}>
	{@render children()}
</nav>

<style>
	.tabs {
		display: flex;
		flex-wrap: wrap;
		border-bottom: 1px solid var(--border_color_30);
		margin-bottom: var(--space_lg);
	}
</style>
