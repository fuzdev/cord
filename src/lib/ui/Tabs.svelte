<script lang="ts">
	/**
	 * A route-driven tab bar: a horizontal `<nav>` around `Tab` children,
	 * each of which derives its own selection from the page's path.
	 *
	 * Deliberately not `role="tablist"`: ARIA tabs imply switching state
	 * within a page, and here each tab is a real route, so plain `<nav>` and
	 * `<a>` semantics match the interaction, and Tab moves through the links
	 * and their controls in order.
	 *
	 * The bar never wraps: past its width it scrolls sideways, so a long
	 * strip stays one row and never widens the page, and the selected tab
	 * scrolls into view (`Tab`). An edge fades while tabs are hidden past
	 * it, re-checked as the bar scrolls, resizes, or its tabs change; the
	 * bar's scroll padding matches the fade, so a tab scrolled into view,
	 * by selection or by focus, lands clear of it.
	 *
	 * Began from visiones' route tabs and grew editor-style tabs (`Tab`'s
	 * preview, double click, and close);
	 * unlike visiones, cord has no global stylesheet, so the look lives here
	 * and in `Tab.svelte`.
	 *
	 * @module
	 */

	import type { Snippet } from 'svelte';
	import type { Attachment } from 'svelte/attachments';

	const {
		children,
		label,
		id
	}: {
		children: Snippet;
		/** The nav's accessible name. */
		label?: string;
		/** The nav's id, for a caller that moves focus into it. */
		id?: string;
	} = $props();

	let hidden_left = $state(false);
	let hidden_right = $state(false);

	const track_hidden: Attachment<HTMLElement> = (el) => {
		const update = (): void => {
			// a pixel's slack, since `scrollLeft` can land fractionally short of its end
			hidden_left = el.scrollLeft > 1;
			hidden_right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
		};
		update();
		el.addEventListener('scroll', update, { passive: true });
		const resized = new ResizeObserver(update);
		resized.observe(el);
		// tabs added, removed, renamed, or restyled (a kept preview loses its
		// italic) change the strip's width but not the bar's
		const changed = new MutationObserver(update);
		changed.observe(el, {
			childList: true,
			subtree: true,
			characterData: true,
			attributes: true,
			attributeFilter: ['class']
		});
		return () => {
			el.removeEventListener('scroll', update);
			resized.disconnect();
			changed.disconnect();
		};
	};
</script>

<nav
	class="tabs"
	class:hidden-left={hidden_left}
	class:hidden-right={hidden_right}
	aria-label={label}
	{id}
	{@attach track_hidden}
>
	{@render children()}
</nav>

<style>
	.tabs {
		/* the offset parent `Tab` measures against when scrolling itself into view */
		position: relative;
		display: flex;
		flex-wrap: nowrap;
		overflow-x: auto;
		overscroll-behavior-x: contain;
		scrollbar-width: thin;
		/* the rule is a shadow, not a border, so the tabs' own underlines
			paint over it inside the scroll box without a negative margin, which
			would make the box scroll vertically too */
		box-shadow: inset 0 -1px 0 var(--border_color_30);
		margin-bottom: var(--space_lg);
		--fade_width: var(--space_xl3);
		--fade_left: 0px;
		--fade_right: 0px;
		scroll-padding-inline: var(--fade_width);
		/* only the mask's alpha counts, so the opaque stop's color is arbitrary */
		mask-image: linear-gradient(
			to right,
			transparent,
			black var(--fade_left),
			black calc(100% - var(--fade_right)),
			transparent
		);
	}

	.hidden-left {
		--fade_left: var(--fade_width);
	}

	.hidden-right {
		--fade_right: var(--fade_width);
	}
</style>
