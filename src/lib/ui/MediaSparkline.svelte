<script lang="ts">
	/**
	 * One measure over the last minute as a small line, for `MediaLoopback`,
	 * with the newest value and the worst, and a readout under the pointer.
	 *
	 * @module
	 */

	const {
		label,
		values,
		format,
		capacity = 60,
		floor_max = 1
	}: {
		label: string;
		/** oldest first, one a second */
		values: Array<number>;
		format: (value: number) => string;
		/** how many values the line spans */
		capacity?: number;
		/** the least the top of the scale may be, so a flat line reads as low */
		floor_max?: number;
	} = $props();

	const WIDTH = 300;
	const HEIGHT = 48;
	const PAD = 3;

	const max = $derived(Math.max(floor_max, ...values));
	const latest = $derived(values.at(-1));
	const to_x = (i: number): number =>
		PAD + ((i + capacity - values.length) / (capacity - 1)) * (WIDTH - PAD * 2);
	const to_y = (v: number): number => PAD + (1 - v / max) * (HEIGHT - PAD * 2);
	const points = $derived(values.map((v, i) => `${to_x(i)},${to_y(v)}`).join(' '));

	let hover_index: number | null = $state.raw(null);

	const on_pointer_move = (e: PointerEvent): void => {
		const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
		const x = ((e.clientX - rect.left) / rect.width) * WIDTH;
		const slot = Math.round(((x - PAD) / (WIDTH - PAD * 2)) * (capacity - 1));
		const i = slot - (capacity - values.length);
		hover_index = i >= 0 && i < values.length ? i : null;
	};
</script>

<figure class="sparkline">
	<figcaption class="row gap_sm font_size_sm">
		<span>{label}</span>
		<span class="font_family_mono text_70">
			{#if hover_index !== null}
				{values.length - 1 - hover_index}s ago: {format(values[hover_index]!)}
			{:else if latest !== undefined}
				now {format(latest)} · worst {format(Math.max(...values))}
			{:else}
				–
			{/if}
		</span>
	</figcaption>
	<svg
		viewBox="0 0 {WIDTH} {HEIGHT}"
		preserveAspectRatio="none"
		role="img"
		aria-label="{label} over the last minute"
		onpointermove={on_pointer_move}
		onpointerleave={() => (hover_index = null)}
	>
		<line
			class="baseline"
			x1={PAD}
			x2={WIDTH - PAD}
			y1={HEIGHT - PAD}
			y2={HEIGHT - PAD}
			vector-effect="non-scaling-stroke"
		/>
		{#if values.length > 1}
			<polyline {points} vector-effect="non-scaling-stroke" />
		{/if}
		{#if hover_index !== null}
			<line
				class="crosshair"
				x1={to_x(hover_index)}
				x2={to_x(hover_index)}
				y1={0}
				y2={HEIGHT}
				vector-effect="non-scaling-stroke"
			/>
		{/if}
	</svg>
</figure>

<style>
	.sparkline {
		margin: 0;
	}
	svg {
		display: block;
		width: 100%;
		height: 48px;
		cursor: crosshair;
	}
	polyline {
		fill: none;
		stroke: var(--palette_b_60);
		stroke-width: 2px;
		stroke-linejoin: round;
		stroke-linecap: round;
	}
	.baseline {
		stroke: var(--ink_20);
		stroke-width: 1px;
	}
	.crosshair {
		stroke: var(--ink_50);
		stroke-width: 1px;
	}
</style>
