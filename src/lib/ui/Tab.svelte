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
	import type { ResolvedPathname } from '$app/types';

	const {
		href,
		selected: selected_override,
		children
	}: {
		/**
		 * The tab's link, from `resolve` in `$app/paths`; passing the route id
		 * with its params (`resolve('/c/[cord_id]/members', {cord_id})`) checks
		 * the subroute, which a template-literal path wouldn't.
		 */
		href: ResolvedPathname;
		/** Overrides the selection derived from the page's path when given. */
		selected?: boolean;
		children: Snippet;
	} = $props();

	const is_exact = $derived(href === page.url.pathname);
	const selected = $derived(
		selected_override ?? (is_exact || page.url.pathname.startsWith(href + '/'))
	);
</script>

<!-- `href` comes resolved from the caller; the rule catches only an inline `resolve` -->
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
		background-color: var(--ink_10);
	}

	.tab.selected {
		border-bottom-color: var(--palette_a_50);
		cursor: default;
	}
</style>
