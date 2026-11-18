<script lang="ts">
	/**
	 * One tab in a `Tabs` bar: a link with `MenuLink`'s selection, `selected`
	 * on its own page and any page under it, with `aria-current="page"` on its
	 * own page only, plus a `selected` override for a tab whose route doesn't
	 * fit that. An index tab is the case here: it sits at its section's root,
	 * so matching descendants would keep it lit on every sibling tab's page,
	 * and its layout passes an exact-only override instead (the
	 * `/c/[cord_id]` overview tab).
	 *
	 * A tab may also be a `preview` tab (italic), take a double click, and
	 * carry a close control after its link, as a cord's space tabs do. The
	 * selected tab scrolls into view in its bar.
	 *
	 * The link is inlined; fuz_ui's `MenuLink` has the same selection and a
	 * `selected` override, so this could wrap it.
	 *
	 * @module
	 */

	import type { Snippet } from 'svelte';
	import type { Attachment } from 'svelte/attachments';
	import { page } from '$app/state';
	import type { ResolvedPathname } from '$app/types';

	const {
		href,
		selected: selected_override,
		preview = false,
		title,
		ondblclick,
		onclose,
		close_label = 'close',
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
		/** Whether this is the preview tab, which opening something else replaces. */
		preview?: boolean;
		/** The link's tooltip. */
		title?: string;
		/** Called on a double click on the link. */
		ondblclick?: () => void;
		/** Shows a close control after the link, which calls this. */
		onclose?: () => void;
		/** The close control's accessible name. */
		close_label?: string;
		children: Snippet;
	} = $props();

	const is_exact = $derived(href === page.url.pathname);
	const selected = $derived(
		selected_override ?? (is_exact || page.url.pathname.startsWith(href + '/'))
	);

	// reruns when `selected` changes; scrolls only the bar, never the page,
	// which `scrollIntoView` would also do, and clears the bar's scroll
	// padding, so the tab lands clear of an edge's fade
	const scroll_into_bar: Attachment<HTMLElement> = (el) => {
		if (!selected) return;
		const bar = el.parentElement;
		if (!bar || el.offsetParent !== bar) return;
		const style = getComputedStyle(bar);
		const left = el.offsetLeft - (parseFloat(style.scrollPaddingLeft) || 0);
		const right = el.offsetLeft + el.offsetWidth + (parseFloat(style.scrollPaddingRight) || 0);
		if (left < bar.scrollLeft) {
			bar.scrollLeft = left;
		} else if (right > bar.scrollLeft + bar.clientWidth) {
			bar.scrollLeft = right - bar.clientWidth;
		}
	};
</script>

<span class="tab" class:selected class:preview {@attach scroll_into_bar}>
	<!-- `href` comes resolved from the caller; the rule catches only an inline `resolve` -->
	<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
	<a
		{href}
		class="tab-link"
		aria-current={is_exact ? 'page' : undefined}
		{title}
		onmousedown={ondblclick
			? (e) => {
					// a double click keeps the tab rather than selecting words; unlike
					// `user-select: none`, this leaves the name selectable
					if (e.detail > 1) e.preventDefault();
				}
			: undefined}
		ondblclick={ondblclick
			? (e) => {
					e.preventDefault();
					ondblclick();
				}
			: undefined}
	>
		{@render children()}
	</a>
	{#if onclose}
		<button
			type="button"
			class="plain tab-close"
			aria-label={close_label}
			title={close_label}
			onclick={onclose}
		>
			×
		</button>
	{/if}
</span>

<style>
	.tab {
		display: inline-flex;
		flex-shrink: 0;
		align-items: center;
		border-bottom: 2px solid transparent;
	}

	.tab:hover {
		background-color: var(--ink_10);
	}

	.tab.selected {
		border-bottom-color: var(--palette_a_50);
	}

	.tab-link {
		display: block;
		max-width: var(--distance_xs);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: var(--space_xs) var(--space_xs2);
		color: inherit;
		text-decoration: none;
	}

	/* the bar's scroll box clips anything past it, so the focus ring draws
		inside the link and the close control rather than around them */
	.tab-link,
	.tab-close {
		outline-offset: calc(-1 * var(--outline_width_focus));
	}

	.selected .tab-link {
		cursor: default;
	}

	.preview .tab-link {
		font-style: italic;
	}

	/* the tab's full height and a width of `--space_xl3`, a target past 24px
		each way, without making the bar any taller */
	.tab-close {
		--button_min_height: 0;
		align-self: stretch;
		min-width: var(--space_xl3);
		padding: 0;
		font-weight: normal;
	}
</style>
