<script lang="ts">
	/**
	 * The mic's live level, its frequencies, the room's noise, a few signs of
	 * trouble, and hearing yourself through the chosen output, for
	 * `MediaCheck`. It reads one stream for its lifetime; key it on the stream.
	 *
	 * @module
	 */

	import { onMount } from 'svelte';
	import { to_error_message } from '@fuzdev/fuz_util/error.ts';

	import { create_media_output, type MediaOutput } from './media_output.ts';
	import MediaSpectrum from './MediaSpectrum.svelte';

	const {
		stream,
		sink_id
	}: {
		stream: MediaStream;
		/** the output device to hear yourself through, empty for the default */
		sink_id: string;
	} = $props();

	// the meter's scale, in dBFS
	const METER_FLOOR_DB = -60;
	const METER_TICKS = [-60, -48, -36, -24, -18, -12, -6, 0];
	// where speech should sit
	const TARGET_LOW_DB = -30;
	const TARGET_HIGH_DB = -12;
	// how fast the smoothed level falls, in dB per second
	const RELEASE_DB_PER_S = 30;
	const PEAK_HOLD_MS = 1500;
	const CLIP_LEVEL = 0.99;
	// a run of exact zeros this long, amid signal, is a gap in the input
	const DROPOUT_MIN_SAMPLES = 96;
	// a frame this late means the page itself stalled
	const STALL_MS = 100;
	// the room's noise and the loudest speech are read over this many readouts
	const HISTORY_READOUTS = 40;
	const READOUT_MS = 250;
	// below any real mic's noise, so only digital silence reads lower
	const SILENCE_DB = -100;

	let level_db = $state.raw(-Infinity);
	let peak_db = $state.raw(-Infinity);
	let clipping = $state.raw(false);
	// the readout updates slower than the bar, so it can be read
	let readout_level_db = $state.raw(-Infinity);
	let readout_peak_db = $state.raw(-Infinity);
	let dropouts = $state.raw(0);
	let stalls = $state.raw(0);
	let clips = $state.raw(0);
	let noise_db = $state.raw(-Infinity);
	let loudest_db = $state.raw(-Infinity);

	let context_info: Array<[string, string]> = $state.raw([]);
	let monitoring = $state.raw(false);
	let output_error: string | null = $state.raw(null);

	let audio_context: AudioContext | null = $state.raw(null);
	let monitor_gain: GainNode | null = $state.raw(null);
	let spectrum_analyser: AnalyserNode | null = $state.raw(null);

	const to_db = (amplitude: number): number => 20 * Math.log10(amplitude);
	const to_percent = (db: number): number =>
		Math.max(0, Math.min(100, ((db - METER_FLOOR_DB) / -METER_FLOOR_DB) * 100));
	const format_db = (db: number): string => (db === -Infinity ? '−∞' : db.toFixed(1));

	const verdict = $derived.by((): { text: string; tone: 'bad' | 'ok' | 'good' } => {
		if (clipping) return { text: 'clipping: turn the mic gain down', tone: 'bad' };
		if (readout_level_db < -55) return { text: 'silent', tone: 'bad' };
		if (readout_level_db < TARGET_LOW_DB - 8) return { text: 'quiet', tone: 'ok' };
		if (readout_level_db <= TARGET_HIGH_DB) return { text: 'good', tone: 'good' };
		return { text: 'loud', tone: 'ok' };
	});

	const toggle_monitor = (): void => {
		if (!monitor_gain || !audio_context) return;
		monitoring = !monitoring;
		monitor_gain.gain.value = monitoring ? 1 : 0;
		void audio_context.resume();
	};

	onMount(() => {
		const track = stream.getAudioTracks()[0];
		if (!track) return;
		// matching the track's rate spares a resample, where the browser allows it
		const track_rate = track.getSettings().sampleRate;
		let ctx: AudioContext;
		try {
			ctx = new AudioContext({ latencyHint: 'interactive', sampleRate: track_rate });
		} catch {
			ctx = new AudioContext({ latencyHint: 'interactive' });
		}
		audio_context = ctx;
		const source = ctx.createMediaStreamSource(stream);
		const analyser = ctx.createAnalyser();
		analyser.fftSize = 2048;
		source.connect(analyser);
		const spectrum = ctx.createAnalyser();
		spectrum.fftSize = 8192;
		spectrum.smoothingTimeConstant = 0.75;
		source.connect(spectrum);
		spectrum_analyser = spectrum;
		// hearing yourself goes through a muted gain until it's turned on; the
		// effect below routes it to the chosen output
		const gain = ctx.createGain();
		gain.gain.value = 0;
		source.connect(gain);
		monitor_gain = gain;

		const update_context_info = (): void => {
			const info: Array<[string, string]> = [
				['sample rate', `${ctx.sampleRate} Hz`],
				['base latency', `${(ctx.baseLatency * 1000).toFixed(1)} ms`]
			];
			if ('outputLatency' in ctx) {
				info.push(['output latency', `${(ctx.outputLatency * 1000).toFixed(1)} ms`]);
			}
			if (track_rate && track_rate !== ctx.sampleRate) {
				info.push(['resampling', `${track_rate} → ${ctx.sampleRate} Hz`]);
			}
			context_info = info;
		};
		update_context_info();

		const samples = new Float32Array(analyser.fftSize);
		let smoothed_db = -Infinity;
		let hold_db = -Infinity;
		let hold_at = 0;
		let clipped_at = -Infinity;
		let last_frame = 0;
		let last_readout = 0;
		// each readout's quietest and loudest level, the newest last
		let window_min_db = Infinity;
		let window_max_db = -Infinity;
		const mins: Array<number> = [];
		const maxes: Array<number> = [];
		let frame: number;
		const tick = (now: number): void => {
			const dt = last_frame ? now - last_frame : 0;
			if (dt > STALL_MS) stalls++;
			last_frame = now;
			analyser.getFloatTimeDomainData(samples);

			let sum = 0;
			let max = 0;
			for (const s of samples) {
				sum += s * s;
				const a = Math.abs(s);
				if (a > max) max = a;
			}
			const rms_db = to_db(Math.sqrt(sum / samples.length));
			// rises at once, falls steadily, so the bar doesn't flicker
			smoothed_db = Math.max(rms_db, smoothed_db - (RELEASE_DB_PER_S * dt) / 1000);
			const frame_peak_db = to_db(max);
			if (frame_peak_db >= hold_db || now - hold_at > PEAK_HOLD_MS) {
				hold_db = frame_peak_db;
				hold_at = now;
			}
			if (max >= CLIP_LEVEL) {
				// one clip per burst, not one per frame
				if (now - clipped_at > READOUT_MS) clips++;
				clipped_at = now;
			}
			window_min_db = Math.min(window_min_db, rms_db);
			window_max_db = Math.max(window_max_db, rms_db);

			// a real mic never reads exact zero, so a run of them amid signal is a
			// gap; only the samples new since the last frame are scanned
			if (max > 0.001) {
				const fresh = Math.min(samples.length, Math.round((dt / 1000) * ctx.sampleRate));
				let run = 0;
				for (let i = samples.length - fresh; i < samples.length; i++) {
					if (samples[i] === 0) {
						run++;
						if (run === DROPOUT_MIN_SAMPLES) dropouts++;
					} else {
						run = 0;
					}
				}
			}

			level_db = smoothed_db;
			peak_db = hold_db;
			clipping = now - clipped_at < 1000;
			if (now - last_readout > READOUT_MS) {
				readout_level_db = smoothed_db;
				readout_peak_db = hold_db;
				last_readout = now;
				mins.push(window_min_db);
				maxes.push(window_max_db);
				if (mins.length > HISTORY_READOUTS) mins.shift();
				if (maxes.length > HISTORY_READOUTS) maxes.shift();
				// digital silence, from a muted or gated source, isn't the room
				const heard = mins.filter((db) => db > SILENCE_DB);
				noise_db = heard.length ? Math.min(...heard) : -Infinity;
				loudest_db = Math.max(...maxes);
				window_min_db = Infinity;
				window_max_db = -Infinity;
				update_context_info();
			}
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);

		return () => {
			cancelAnimationFrame(frame);
			void ctx.close();
			audio_context = null;
			monitor_gain = null;
			spectrum_analyser = null;
		};
	});

	// routes hearing yourself to the chosen output, again on each change
	$effect(() => {
		const ctx = audio_context;
		const gain = monitor_gain;
		const id = sink_id;
		if (!ctx || !gain) return;
		let output: MediaOutput | null = null;
		let cancelled = false;
		output_error = null;
		create_media_output(ctx, id).then(
			(next) => {
				if (cancelled) return next.close();
				output = next;
				gain.connect(next.node);
			},
			(err: unknown) => {
				if (!cancelled) output_error = to_error_message(err);
			}
		);
		return () => {
			cancelled = true;
			if (!output) return;
			try {
				gain.disconnect(output.node);
			} catch {
				// the context already closed
			}
			output.close();
		};
	});

	const speech_gap_db = $derived(loudest_db - noise_db);
</script>

<div class="meter" class:clipping aria-hidden="true">
	<div class="meter-peak" style:width="{to_percent(peak_db)}%"></div>
	<div class="meter-level" style:width="{to_percent(level_db)}%"></div>
	<!-- over the fills, so the band shows at any level -->
	<div
		class="meter-target"
		style:left="{to_percent(TARGET_LOW_DB)}%"
		style:width="{to_percent(TARGET_HIGH_DB) - to_percent(TARGET_LOW_DB)}%"
	></div>
</div>
<div class="meter-scale font_size_xs text_50" aria-hidden="true">
	{#each METER_TICKS as tick (tick)}
		<span style:left="{to_percent(tick)}%">{tick}</span>
	{/each}
</div>
<p>
	<strong class={['verdict', verdict.tone]}>{verdict.text}</strong>
	<span class="font_family_mono font_size_sm text_70">
		· level {format_db(readout_level_db)} dBFS · peak {format_db(readout_peak_db)} dBFS
	</span>
</p>
<p class="text_50 font_size_sm">
	Talk normally: the solid bar should sit in the shaded band, and the lighter peak shouldn’t reach
	the right edge.
</p>
{#if spectrum_analyser}<MediaSpectrum analyser={spectrum_analyser} />{/if}
<table class="font_size_sm">
	<tbody>
		<tr>
			<th>room noise</th>
			<td>
				<span class="font_family_mono">{format_db(noise_db)} dBFS</span>
				<span class="text_50">(the quietest moment of the last ten seconds)</span>
			</td>
		</tr>
		<tr>
			<th>loudest</th>
			<td>
				<span class="font_family_mono">{format_db(loudest_db)} dBFS</span>
				{#if Number.isFinite(speech_gap_db)}
					<span class="text_50">
						({speech_gap_db.toFixed(0)} dB above the room; 30 or more is clear speech)
					</span>
				{/if}
			</td>
		</tr>
		<tr>
			<th>clips</th>
			<td>
				{clips}
				<span class="text_50">(times the signal hit full scale; should stay 0)</span>
			</td>
		</tr>
		<tr>
			<th>dropouts</th>
			<td>
				{dropouts}
				<span class="text_50">(gaps of silence cut into the input; should stay 0)</span>
			</td>
		</tr>
		<tr>
			<th>page stalls</th>
			<td>
				{stalls}
				<span class="text_50">(the page froze over {STALL_MS} ms; a busy machine shows here)</span>
			</td>
		</tr>
		{#each context_info as [key, value] (key)}
			<tr>
				<th>{key}</th>
				<td class="font_family_mono">{value}</td>
			</tr>
		{/each}
	</tbody>
</table>
<button type="button" onclick={toggle_monitor}>
	{monitoring ? 'stop hearing yourself' : 'hear yourself'}
</button>
{#if output_error}<p class="color_c_50">{output_error}</p>{/if}
{#if monitoring}
	<p class="text_50 font_size_sm">
		Use headphones, or the speakers feed back. This plays at the lowest delay the browser manages,
		so it’s the first thing to break up when audio is struggling; if it’s choppy, try a recording
		below: a clean recording means the mic is fine and live playback is the trouble. Echo
		cancellation can also cut into hearing yourself, so try it with that off.
	</p>
{/if}

<style>
	.meter {
		position: relative;
		height: var(--space_xl2);
		background: var(--ink_10);
		border-radius: var(--border_radius_xs);
		overflow: hidden;
	}
	.meter > div {
		position: absolute;
		top: 0;
		bottom: 0;
		left: 0;
	}
	.meter-target {
		border-inline: 2px dashed var(--ink_50);
		background: var(--ink_05);
	}
	.meter-peak {
		background: var(--palette_b_30);
	}
	.meter-level {
		background: var(--palette_b_60);
	}
	.meter.clipping .meter-peak {
		background: var(--palette_c_50);
	}
	.meter-scale {
		position: relative;
		height: 1.5em;
	}
	.meter-scale span {
		position: absolute;
		transform: translateX(-50%);
	}
	.meter-scale span:first-child {
		transform: none;
	}
	.meter-scale span:last-child {
		transform: translateX(-100%);
	}
	.verdict.good {
		color: var(--palette_b_60);
	}
	.verdict.ok {
		color: var(--palette_e_60);
	}
	.verdict.bad {
		color: var(--palette_c_60);
	}
</style>
