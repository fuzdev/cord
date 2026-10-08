<script lang="ts">
	/**
	 * One tab in a `Tabs` bar: an `<a class="tab">` with `MenuLink`'s
	 * selection, `selected` on its own page and any page under it, with
	 * `aria-current="page"` on its own page only, plus a `selected` override
	 * for a tab whose route doesn't fit that. An index tab is the case here:
	 * it sits at its section's root, so matching descendants would keep it lit
	 * on every sibling tab's page, and its layout passes an exact-only
	 * override instead (the `/c/[cord_id]` overview tab).
	 *
	 * The `<a>` is inlined rather than wrapping `MenuLink`, which derives
	 * `selected` itself with no override.
	 *
	 * @module
	 */

	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import type { Pathname } from '$app/types';

	const {
		path,
		selected: selected_override,
		children
	}: {
		/**
		 * The pathname passed to `resolve` from `$app/paths` for the `href`.
		 * Under a dynamic segment SvelteKit's `Pathname` ends in a `${string}`
		 * that accepts any tail, so it doesn't catch a misspelled subroute.
		 */
		path: Pathname;
		/** Overrides the selection derived from the page's path when given. */
		selected?: boolean;
		children: Snippet;
	} = $props();

	const href = $derived(resolve(path));
	const is_exact = $derived(href === page.url.pathname);
	const selected = $derived(
		selected_override ?? (is_exact || page.url.pathname.startsWith(href + '/'))
	);
</script>

<!-- `href` is `resolve(path)` above; the rule catches only an inline `resolve` -->
<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
<a {href} class="tab" class:selected aria-current={is_exact ? 'page' : undefined}>
	{@render children()}
</a>

<style>
	.tab {
		display: inline-flex;
		align-items: center;
		padding: var(--space_xs) var(--space_xs2);
		border-bottom: 2px solid transparent;
		border-radius: 0;
		margin-bottom: -1px;
		color: inherit;
		text-decoration: none;
	}

	.tab:hover {
		background-color: var(--fg_10);
	}

	.tab.selected {
		border-bottom-color: var(--palette_a_50);
		cursor: default;
	}
</style>
