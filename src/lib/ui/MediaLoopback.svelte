<script lang="ts">
	/**
	 * A WebRTC call to itself, for `MediaCheck`: the stream goes through two
	 * peer connections in this page and plays back as a call would, with the
	 * receiver's stats each second. The network is perfect here, so trouble
	 * in these numbers comes from this machine's audio or load, not a
	 * connection. It reads one stream for its lifetime; key it on the stream.
	 *
	 * @module
	 */

	import { onDestroy } from 'svelte';
	import { to_error_message } from '@fuzdev/fuz_util/error.ts';

	import { set_media_sink } from './media_output.ts';
	import MediaSparkline from './MediaSparkline.svelte';

	const {
		stream,
		sink_id
	}: {
		stream: MediaStream;
		/** the output device to listen through, empty for the default */
		sink_id: string;
	} = $props();

	// a minute of history, one value a second
	const HISTORY_SECONDS = 60;
	const push_history = (list: Array<number>, value: number): Array<number> =>
		[...list, value].slice(-HISTORY_SECONDS);

	const has_video = $derived(stream.getVideoTracks().length > 0);

	type StatsRow = [label: string, value: string, note?: string];

	let running = $state.raw(false);
	let error: string | null = $state.raw(null);
	let remote: MediaStream | null = $state.raw(null);
	let listening = $state.raw(false);
	let audio_rows: Array<StatsRow> = $state.raw([]);
	let video_rows: Array<StatsRow> = $state.raw([]);
	let concealed_history: Array<number> = $state.raw([]);
	let buffer_history: Array<number> = $state.raw([]);
	let sink_error: string | null = $state.raw(null);

	let remote_el: HTMLVideoElement | HTMLAudioElement | undefined = $state();

	let sender: RTCPeerConnection | null = null;
	let receiver: RTCPeerConnection | null = null;
	let timer: ReturnType<typeof setInterval> | null = null;

	// the stats types are wide and vary by browser, so fields are read loosely
	type Stats = Record<string, unknown>;
	const num = (s: Stats | undefined, key: string): number | null => {
		const v = s?.[key];
		return typeof v === 'number' ? v : null;
	};
	const ms = (seconds: number | null): string =>
		seconds === null ? '–' : `${(seconds * 1000).toFixed(1)} ms`;
	const pct = (part: number | null, whole: number | null): string =>
		part === null || !whole ? '–' : `${((part / whole) * 100).toFixed(2)}%`;
	const delta = (now: Stats | undefined, prev: Stats | undefined, key: string): number | null => {
		const a = num(now, key);
		const b = num(prev, key);
		return a === null ? null : a - (b ?? 0);
	};
	const to_codec = (report: RTCStatsReport, s: Stats | undefined): string => {
		const codec = report.get(String(s?.codecId)) as Stats | undefined;
		if (!codec) return '–';
		const channels = num(codec, 'channels');
		return `${String(codec.mimeType)} ${num(codec, 'clockRate')} Hz${channels ? ` ×${channels}` : ''}`;
	};
	const to_kbps = (bytes: number | null, seconds: number): string =>
		bytes === null ? '–' : `${((bytes * 8) / 1000 / seconds).toFixed(0)} kbps`;

	let prev_audio: Stats | undefined;
	let prev_video: Stats | undefined;
	let prev_playout: Stats | undefined;
	let prev_at = 0;

	const read_stats = async (): Promise<void> => {
		if (!receiver || !sender) return;
		const [report, sent] = await Promise.all([receiver.getStats(), sender.getStats()]);
		const now = performance.now();
		const seconds = prev_at ? (now - prev_at) / 1000 : 1;
		prev_at = now;

		let audio: Stats | undefined;
		let video: Stats | undefined;
		let playout: Stats | undefined;
		let rtt: number | null = null;
		for (const s of report.values() as Iterable<Stats>) {
			if (s.type === 'inbound-rtp' && s.kind === 'audio') audio = s;
			else if (s.type === 'inbound-rtp' && s.kind === 'video') video = s;
			else if (s.type === 'media-playout') playout = s;
			else if (s.type === 'candidate-pair' && s.nominated) rtt = num(s, 'currentRoundTripTime');
		}
		let limitation: string | null = null;
		for (const s of sent.values() as Iterable<Stats>) {
			if (s.type === 'outbound-rtp' && s.kind === 'video') {
				limitation =
					typeof s.qualityLimitationReason === 'string' ? s.qualityLimitationReason : null;
			}
		}

		if (audio) {
			const samples = delta(audio, prev_audio, 'totalSamplesReceived');
			const emitted = delta(audio, prev_audio, 'jitterBufferEmittedCount');
			const buffer_delay = delta(audio, prev_audio, 'jitterBufferDelay');
			const concealed = delta(audio, prev_audio, 'concealedSamples');
			// the first read has no second before it to compare against
			if (prev_audio) {
				if (concealed !== null && samples) {
					concealed_history = push_history(concealed_history, (concealed / samples) * 100);
				}
				if (buffer_delay !== null && emitted) {
					buffer_history = push_history(buffer_history, (buffer_delay / emitted) * 1000);
				}
			}
			const rows: Array<StatsRow> = [
				['codec', to_codec(report, audio)],
				['bitrate', to_kbps(delta(audio, prev_audio, 'bytesReceived'), seconds)],
				[
					'concealed, last second',
					pct(delta(audio, prev_audio, 'concealedSamples'), samples),
					'audio the receiver had to make up; above about 1% is audible'
				],
				[
					'concealed, total',
					pct(num(audio, 'concealedSamples'), num(audio, 'totalSamplesReceived')),
					`${num(audio, 'concealmentEvents') ?? 0} events`
				],
				[
					'stretched / squeezed',
					`${pct(delta(audio, prev_audio, 'insertedSamplesForDeceleration'), samples)} / ${pct(delta(audio, prev_audio, 'removedSamplesForAcceleration'), samples)}`,
					'speed changes to fit the buffer; heavy use sounds warbly'
				],
				[
					'jitter buffer',
					buffer_delay !== null && emitted ? ms(buffer_delay / emitted) : '–',
					'how far behind it plays to stay smooth'
				],
				['network jitter', ms(num(audio, 'jitter')), 'near 0 on a loopback'],
				[
					'packets lost',
					`${num(audio, 'packetsLost') ?? 0} of ${num(audio, 'packetsReceived') ?? 0}`
				]
			];
			if (playout) {
				const playout_samples = delta(playout, prev_playout, 'totalSamplesCount');
				const playout_delay = delta(playout, prev_playout, 'totalPlayoutDelay');
				rows.push(
					[
						'playout gaps, total',
						`${(num(playout, 'synthesizedSamplesDuration') ?? 0).toFixed(2)} s`,
						`${num(playout, 'synthesizedSamplesEvents') ?? 0} events; silence the output had to fill, a sign the audio device is starved`
					],
					[
						'playout delay',
						playout_delay !== null && playout_samples ? ms(playout_delay / playout_samples) : '–'
					]
				);
			}
			rows.push(['round trip', ms(rtt)]);
			audio_rows = rows;
		}
		if (video) {
			video_rows = [
				['codec', to_codec(report, video)],
				['bitrate', to_kbps(delta(video, prev_video, 'bytesReceived'), seconds)],
				['size', `${num(video, 'frameWidth') ?? '–'}×${num(video, 'frameHeight') ?? '–'}`],
				['frame rate', `${num(video, 'framesPerSecond') ?? '–'} fps`],
				['frames dropped', String(num(video, 'framesDropped') ?? 0)],
				[
					'freezes',
					`${num(video, 'freezeCount') ?? 0} (${(num(video, 'totalFreezesDuration') ?? 0).toFixed(1)} s)`
				],
				[
					'sender limited by',
					limitation ?? '–',
					'"cpu" means the encoder can’t keep up on this machine'
				]
			];
		}
		prev_audio = audio;
		prev_video = video;
		prev_playout = playout;
	};

	const start = async (): Promise<void> => {
		stop();
		error = null;
		running = true;
		try {
			const a = new RTCPeerConnection();
			const b = new RTCPeerConnection();
			sender = a;
			receiver = b;
			a.addEventListener('icecandidate', (e) => {
				if (e.candidate) void b.addIceCandidate(e.candidate);
			});
			b.addEventListener('icecandidate', (e) => {
				if (e.candidate) void a.addIceCandidate(e.candidate);
			});
			const incoming = new MediaStream();
			b.addEventListener('track', (e) => incoming.addTrack(e.track));
			for (const track of stream.getTracks()) a.addTrack(track, stream);
			const offer = await a.createOffer();
			await a.setLocalDescription(offer);
			await b.setRemoteDescription(offer);
			const answer = await b.createAnswer();
			await b.setLocalDescription(answer);
			await a.setRemoteDescription(answer);
			remote = incoming;
			timer = setInterval(() => void read_stats(), 1000);
		} catch (err) {
			error = to_error_message(err);
			stop();
		}
	};

	const stop = (): void => {
		if (timer !== null) clearInterval(timer);
		timer = null;
		sender?.close();
		receiver?.close();
		sender = null;
		receiver = null;
		remote = null;
		running = false;
		listening = false;
		audio_rows = [];
		video_rows = [];
		concealed_history = [];
		buffer_history = [];
		prev_audio = prev_video = prev_playout = undefined;
		prev_at = 0;
	};

	// the element stays unmuted so the browser keeps pulling the audio, as it
	// would in a call; listening only sets the volume
	$effect(() => {
		if (!remote_el) return;
		remote_el.srcObject = remote;
		remote_el.volume = listening ? 1 : 0;
	});

	$effect(() => {
		if (!remote_el) return;
		sink_error = null;
		set_media_sink(remote_el, sink_id).catch((err: unknown) => {
			sink_error = to_error_message(err);
		});
	});

	onDestroy(stop);
</script>

<div class="row gap_sm">
	{#if running}
		<button type="button" onclick={stop}>stop the loopback</button>
		<button type="button" onclick={() => (listening = !listening)}>
			{listening ? 'stop listening' : 'listen'}
		</button>
	{:else}
		<button type="button" onclick={start}>start a loopback call</button>
	{/if}
</div>
{#if error}<p class="color_c_50">{error}</p>{/if}
{#if remote}
	{#if has_video}
		<video bind:this={remote_el} class="preview" autoplay playsinline></video>
	{:else}
		<audio bind:this={remote_el} autoplay></audio>
	{/if}
	{#if listening}
		<p class="text_50 font_size_sm">
			Headphones, please. This is how a call’s audio would reach the other side, after the codec and
			jitter buffer.
		</p>
	{/if}
	{#if sink_error}<p class="color_c_50">{sink_error}</p>{/if}
	{#if audio_rows.length}
		<div class="sparklines">
			<MediaSparkline
				label="concealed audio"
				values={concealed_history}
				format={(v) => `${v.toFixed(2)}%`}
				floor_max={2}
			/>
			<MediaSparkline
				label="jitter buffer"
				values={buffer_history}
				format={(v) => `${v.toFixed(0)} ms`}
				floor_max={100}
			/>
		</div>
		{@render stats_table('audio received', audio_rows)}
	{:else}
		<p class="text_50">connecting…</p>
	{/if}
	{#if video_rows.length}
		{@render stats_table('video received', video_rows)}
	{/if}
	<p class="text_50 font_size_sm">
		Let it run a minute while you talk. In Chromium, <code>chrome://webrtc-internals</code> shows
		the same numbers, graphed, for any call in the browser.
	</p>
{:else if !running}
	<p class="text_50 font_size_sm">
		Sends your mic and camera through WebRTC to this same page, as a call would. With no network in
		the way, concealed audio or playout gaps here mean this machine is the trouble.
	</p>
{/if}

{#snippet stats_table(caption: string, rows: Array<StatsRow>)}
	<table class="font_size_sm">
		<caption class="text-align:left">{caption}</caption>
		<tbody>
			{#each rows as [label, value, note] (label)}
				<tr>
					<th>{label}</th>
					<td class="font_family_mono">{value}</td>
					<td class="text_50">{note ?? ''}</td>
				</tr>
			{/each}
		</tbody>
	</table>
{/snippet}

<style>
	.sparklines {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
		gap: var(--space_md);
	}
	.preview {
		display: block;
		width: 100%;
		max-height: 40vh;
		object-fit: contain;
		border-radius: var(--border_radius_xs);
	}
</style>
