<script lang="ts">
	import Svg from '@fuzdev/fuz_ui/Svg.svelte';

	import { LOGO_COLORS, logo_ideas, to_logo_svg_data } from './logo_ideas.ts';

	// the sizes a logo has to survive: a page header, a nav entry, a favicon
	const SIZES = ['128px', '48px', '16px'];

	let color = $state(LOGO_COLORS[0]!.value);
</script>

<svelte:head>
	<title>logo lab - Cord</title>
</svelte:head>

<main class="width_atmost_lg mx_auto p_lg">
	<h1>logo lab</h1>
	<p>
		Ideas for Cord's logo, none chosen. Each is one color on a square, drawn the way the other Fuz
		logos are, and shown at the sizes a logo has to survive.
	</p>
	<section>
		<h2>color</h2>
		<div class="row gap_xs flex-wrap:wrap">
			{#each LOGO_COLORS as logo_color (logo_color.value)}
				<button
					type="button"
					class:selected={color === logo_color.value}
					title="worn by {logo_color.worn_by}"
					onclick={() => (color = logo_color.value)}
				>
					<span class="swatch" style:background-color={logo_color.value}></span>
					{logo_color.name}
				</button>
			{/each}
		</div>
	</section>
	<section>
		<h2>ideas</h2>
		<ul class="unstyled ideas">
			{#each logo_ideas as idea (idea.name)}
				{@const data = to_logo_svg_data(idea, color)}
				<li class="panel p_md">
					<h3>{idea.name}</h3>
					<div class="row gap_md">
						{#each SIZES as size (size)}
							<Svg {data} {size} shrink={false} />
						{/each}
					</div>
					<p><small>{idea.label}</small></p>
					<p>{idea.note}</p>
				</li>
			{/each}
		</ul>
	</section>
</main>

<style>
	.ideas {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: var(--space_md);
	}
	.swatch {
		display: inline-block;
		width: var(--space_md);
		height: var(--space_md);
		border-radius: 50%;
		margin-right: var(--space_xs);
	}
</style>
