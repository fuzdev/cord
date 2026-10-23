<script lang="ts">
	/**
	 * A local check of the mic, camera, and screen share: the camera on
	 * screen, a live mic level, a recording to play back, a WebRTC call to
	 * itself, the devices the browser offers, and what each track actually
	 * got. Nothing leaves the browser, so it needs no account.
	 *
	 * Browser APIs run only from the mount and the buttons, so the page
	 * prerenders.
	 *
	 * @module
	 */

	import { onMount } from 'svelte';
	import { to_error_message } from '@fuzdev/fuz_util/error.ts';

	import MediaMic from './MediaMic.svelte';
	import MediaRecording from './MediaRecording.svelte';
	import MediaLoopback from './MediaLoopback.svelte';
	import MediaSpeakerTest from './MediaSpeakerTest.svelte';
	import { can_choose_output } from './media_output.ts';

	const RESOLUTIONS = [
		{ label: '640×480', width: 640, height: 480 },
		{ label: '1280×720', width: 1280, height: 720 },
		{ label: '1920×1080', width: 1920, height: 1080 },
		{ label: '3840×2160', width: 3840, height: 2160 }
	];
	const ASPECT_RATIOS = [
		{ label: '4:3', value: 4 / 3 },
		{ label: '16:9', value: 16 / 9 }
	];
	const FRAME_RATES = [15, 24, 30, 60];

	// read on mount, since the page prerenders
	let supported = $state.raw(true);
	let secure = $state.raw(true);
	let has_display_media = $state.raw(false);
	let has_webrtc = $state.raw(false);
	let can_output = $state.raw(false);
	let can_pick_output = $state.raw(false);
	let browser_rows: Array<[string, string]> = $state.raw([]);

	// the output device for everything this page plays, empty for the default
	let sink_id = $state('');
	let output_error: string | null = $state.raw(null);

	let devices: Array<MediaDeviceInfo> = $state.raw([]);
	const audio_inputs = $derived(devices.filter((d) => d.kind === 'audioinput'));
	const video_inputs = $derived(devices.filter((d) => d.kind === 'videoinput'));
	const audio_outputs = $derived(devices.filter((d) => d.kind === 'audiooutput'));

	let audio_input_id = $state('');
	let video_input_id = $state('');
	let want_audio = $state(true);
	let want_video = $state(true);
	let echo_cancellation = $state(true);
	let noise_suppression = $state(true);
	let auto_gain_control = $state(true);
	// indexes into the lists above, or -1 for the camera's own choice
	let resolution_index = $state(-1);
	let aspect_index = $state(-1);
	let frame_rate = $state(0);
	let video_error: string | null = $state.raw(null);

	let mic_permission: PermissionState | 'unknown' = $state.raw('unknown');
	let camera_permission: PermissionState | 'unknown' = $state.raw('unknown');

	let stream: MediaStream | null = $state.raw(null);
	let starting = $state.raw(false);
	let error: string | null = $state.raw(null);

	let audio_track_info: TrackInfo | null = $state.raw(null);
	let video_track_info: TrackInfo | null = $state.raw(null);

	// the camera's measured frames
	let video_el: HTMLVideoElement | undefined = $state();
	let video_size = $state.raw('');
	let fps: number | null = $state.raw(null);

	let screen_stream: MediaStream | null = $state.raw(null);
	let screen_el: HTMLVideoElement | undefined = $state();
	let screen_info: TrackInfo | null = $state.raw(null);
	let screen_error: string | null = $state.raw(null);

	// the camera's average brightness, 0 to 1
	let brightness: number | null = $state.raw(null);
	const DIM_BRIGHTNESS = 0.25;

	interface TrackInfo {
		label: string;
		state: string;
		muted: boolean;
		settings: Array<[string, string]>;
		capabilities: Array<[string, string]>;
	}

	const HIDDEN_KEYS = new Set(['deviceId', 'groupId']);
	const format_setting = (value: unknown): string => {
		if (Array.isArray(value)) return value.join(', ');
		if (typeof value === 'number') return String(Math.round(value * 1000) / 1000);
		if (value && typeof value === 'object') {
			// a capability range
			const { min, max } = value as { min?: number; max?: number };
			return `${format_setting(min)} – ${format_setting(max)}`;
		}
		return String(value);
	};
	const to_rows = (record: object): Array<[string, string]> =>
		Object.entries(record)
			.filter(([key]) => !HIDDEN_KEYS.has(key))
			.map(([key, value]) => [key, format_setting(value)]);

	const to_track_info = (track: MediaStreamTrack): TrackInfo => ({
		label: track.label,
		state: track.readyState,
		muted: track.muted,
		settings: to_rows(track.getSettings()),
		// Firefox has no `getCapabilities`
		capabilities: 'getCapabilities' in track ? to_rows(track.getCapabilities()) : []
	});

	// Linux lists a "Monitor of" input for every output: it records what plays
	const is_output_monitor = (d: MediaDeviceInfo): boolean => d.label.startsWith('Monitor of ');

	const to_video_constraints = (): MediaTrackConstraints => {
		const resolution = RESOLUTIONS[resolution_index];
		const aspect = ASPECT_RATIOS[aspect_index];
		return {
			deviceId: video_input_id ? { exact: video_input_id } : undefined,
			width: resolution ? { ideal: resolution.width } : undefined,
			height: resolution ? { ideal: resolution.height } : undefined,
			aspectRatio: aspect ? { ideal: aspect.value } : undefined,
			frameRate: frame_rate ? { ideal: frame_rate } : undefined
		};
	};

	// the camera's size and rate change live, without a restart
	const apply_video_constraints = async (): Promise<void> => {
		const track = stream?.getVideoTracks()[0];
		if (!track) return;
		video_error = null;
		const { deviceId: _, ...constraints } = to_video_constraints();
		try {
			await track.applyConstraints(constraints);
		} catch (err) {
			video_error = to_media_error_message(err);
		}
		fps = null;
		update_track_info();
	};

	const refresh_devices = async (): Promise<void> => {
		devices = await navigator.mediaDevices.enumerateDevices();
	};

	const refresh_permissions = async (): Promise<void> => {
		// not every browser can query these, so a refusal just stays unknown
		const query = async (name: string): Promise<PermissionState | 'unknown'> => {
			try {
				return (await navigator.permissions.query({ name: name as PermissionName })).state;
			} catch {
				return 'unknown';
			}
		};
		[mic_permission, camera_permission] = await Promise.all([query('microphone'), query('camera')]);
	};

	const stop = (): void => {
		for (const track of stream?.getTracks() ?? []) track.stop();
		stream = null;
		audio_track_info = null;
		video_track_info = null;
		video_error = null;
		video_size = '';
		fps = null;
		brightness = null;
	};

	const start = async (): Promise<void> => {
		if (starting) return;
		stop();
		error = null;
		if (!want_audio && !want_video) {
			error = 'Pick the mic, the camera, or both.';
			return;
		}
		starting = true;
		try {
			const next = await navigator.mediaDevices.getUserMedia({
				audio: want_audio
					? {
							deviceId: audio_input_id ? { exact: audio_input_id } : undefined,
							echoCancellation: echo_cancellation,
							noiseSuppression: noise_suppression,
							autoGainControl: auto_gain_control
						}
					: false,
				video: want_video ? to_video_constraints() : false
			});
			stream = next;
			const audio_track = next.getAudioTracks()[0];
			const video_track = next.getVideoTracks()[0];
			if (audio_track) audio_input_id = audio_track.getSettings().deviceId ?? '';
			if (video_track) video_input_id = video_track.getSettings().deviceId ?? '';
			for (const track of next.getTracks()) {
				// a device unplugged, or a permission pulled, ends the track
				track.addEventListener('ended', update_track_info);
				track.addEventListener('mute', update_track_info);
				track.addEventListener('unmute', update_track_info);
			}
			update_track_info();
			// labels arrive only once a permission is granted
			await Promise.all([refresh_devices(), refresh_permissions()]);
		} catch (err) {
			error = to_media_error_message(err);
			await refresh_permissions();
		} finally {
			starting = false;
		}
	};

	const update_track_info = (): void => {
		const audio_track = stream?.getAudioTracks()[0];
		const video_track = stream?.getVideoTracks()[0];
		audio_track_info = audio_track ? to_track_info(audio_track) : null;
		video_track_info = video_track ? to_track_info(video_track) : null;
	};

	const to_media_error_message = (err: unknown): string => {
		if (!(err instanceof DOMException)) return to_error_message(err);
		switch (err.name) {
			case 'NotAllowedError':
				return 'Permission was refused. Allow the mic and camera for this site in the browser, then try again.';
			case 'NotFoundError':
				return 'No matching device was found.';
			case 'NotReadableError':
				return 'The device is there but couldn’t be opened; another app may be holding it.';
			case 'OverconstrainedError':
				return 'The chosen device couldn’t be opened with these settings; try another.';
			default:
				return `${err.name}: ${err.message}`;
		}
	};

	const start_screen = async (): Promise<void> => {
		stop_screen();
		screen_error = null;
		try {
			const next = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
			screen_stream = next;
			const track = next.getVideoTracks()[0];
			if (track) {
				screen_info = to_track_info(track);
				// the browser's own "stop sharing" ends the track
				track.addEventListener('ended', stop_screen);
			}
		} catch (err) {
			screen_error = to_media_error_message(err);
		}
	};

	const stop_screen = (): void => {
		for (const track of screen_stream?.getTracks() ?? []) track.stop();
		screen_stream = null;
		screen_info = null;
	};

	$effect(() => {
		if (video_el) video_el.srcObject = stream;
	});
	$effect(() => {
		if (screen_el) screen_el.srcObject = screen_stream;
	});

	// counts the frames the camera actually delivers, where the browser can
	$effect(() => {
		const el = video_el;
		if (!el || !stream?.getVideoTracks().length) return;
		if (!('requestVideoFrameCallback' in el)) return;
		let handle: number;
		let window_start = 0;
		let window_frames = 0;
		const on_frame = (now: number): void => {
			video_size = `${el.videoWidth}×${el.videoHeight}`;
			if (!window_start) window_start = now;
			window_frames++;
			if (now - window_start >= 1000) {
				fps = Math.round((window_frames * 1000) / (now - window_start));
				window_start = now;
				window_frames = 0;
			}
			handle = el.requestVideoFrameCallback(on_frame);
		};
		handle = el.requestVideoFrameCallback(on_frame);
		return () => el.cancelVideoFrameCallback(handle);
	});

	const selected_label = (list: Array<MediaDeviceInfo>, id: string): string =>
		list.find((d) => d.deviceId === id)?.label || '(unnamed)';

	// Firefox asks the viewer for an output it hasn't listed yet
	type PickOutput = () => Promise<MediaDeviceInfo>;
	const to_pick_output = (): PickOutput | undefined => {
		const devices = navigator.mediaDevices as MediaDevices & { selectAudioOutput?: PickOutput };
		return devices.selectAudioOutput?.bind(devices);
	};
	const pick_output = async (): Promise<void> => {
		output_error = null;
		try {
			const device = await to_pick_output()?.();
			if (!device) return;
			await refresh_devices();
			sink_id = device.deviceId;
		} catch (err) {
			output_error = to_media_error_message(err);
		}
	};

	// samples a tiny copy of the frame twice a second
	$effect(() => {
		const el = video_el;
		if (!el || !stream?.getVideoTracks().length) return;
		const canvas = document.createElement('canvas');
		canvas.width = 32;
		canvas.height = 18;
		const g = canvas.getContext('2d', { willReadFrequently: true });
		if (!g) return;
		const timer = setInterval(() => {
			if (el.readyState < 2) return;
			g.drawImage(el, 0, 0, canvas.width, canvas.height);
			const { data } = g.getImageData(0, 0, canvas.width, canvas.height);
			let sum = 0;
			for (let i = 0; i < data.length; i += 4) {
				sum += 0.2126 * data[i]! + 0.7152 * data[i + 1]! + 0.0722 * data[i + 2]!;
			}
			brightness = sum / (data.length / 4) / 255;
		}, 500);
		return () => clearInterval(timer);
	});

	const BROWSERS: Array<[RegExp, string]> = [
		[/Edg\/([\d.]+)/, 'Edge'],
		[/Firefox\/([\d.]+)/, 'Firefox'],
		[/Chrome\/([\d.]+)/, 'Chromium'],
		[/Version\/([\d.]+).*Safari/, 'Safari']
	];
	const to_browser_name = (ua: string): string => {
		for (const [pattern, name] of BROWSERS) {
			const match = pattern.exec(ua);
			if (match) return `${name} ${match[1]}`;
		}
		return ua;
	};
	// the codecs a call can send, without the helpers that ride along
	const CODEC_HELPERS = new Set(['rtx', 'red', 'ulpfec', 'flexfec-03', 'telephone-event', 'cn']);
	const to_codecs = (kind: 'audio' | 'video'): string => {
		const codecs = RTCRtpSender.getCapabilities(kind)?.codecs ?? [];
		const names = codecs
			.map((c) => c.mimeType.split('/')[1] ?? c.mimeType)
			.filter((name) => !CODEC_HELPERS.has(name.toLowerCase()));
		return [...new Set(names)].join(', ') || '–';
	};
	const to_browser_rows = (): Array<[string, string]> => {
		const rows: Array<[string, string]> = [
			['browser', to_browser_name(navigator.userAgent)],
			['CPU threads', String(navigator.hardwareConcurrency || '–')]
		];
		// Chromium's own estimate of the connection
		const connection = (
			navigator as Navigator & {
				connection?: { effectiveType?: string; downlink?: number; rtt?: number };
			}
		).connection;
		if (connection?.effectiveType) {
			rows.push([
				'connection',
				`${connection.effectiveType} · ~${connection.downlink} Mbps · ~${connection.rtt} ms (the browser’s estimate)`
			]);
		}
		if ('RTCRtpSender' in window && 'getCapabilities' in RTCRtpSender) {
			rows.push(['audio codecs', to_codecs('audio')], ['video codecs', to_codecs('video')]);
		}
		return rows;
	};

	onMount(() => {
		secure = window.isSecureContext;
		can_output = can_choose_output();
		can_pick_output = !!to_pick_output();
		browser_rows = to_browser_rows();
		supported = !!navigator.mediaDevices?.getUserMedia;
		has_display_media = !!navigator.mediaDevices?.getDisplayMedia;
		has_webrtc = 'RTCPeerConnection' in window;
		if (!supported) return;
		void refresh_devices();
		void refresh_permissions();
		const on_device_change = (): void => void refresh_devices();
		navigator.mediaDevices.addEventListener('devicechange', on_device_change);
		return () => {
			navigator.mediaDevices.removeEventListener('devicechange', on_device_change);
			stop();
			stop_screen();
		};
	});
</script>

{#if !supported}
	<p>
		This browser doesn’t offer the mic or camera here.
		{#if !secure}It needs a secure page: <code>https://</code> or <code>localhost</code>.{/if}
	</p>
{:else}
	<div class="av-layout">
		<div class="av-grid">
			<div class="av-column">
				<!-- the feed sits flush to the panel's edges, the text below padded -->
				<section class="panel camera-panel" aria-label="camera">
					{#if video_track_info}
						<!-- mirrored, as a self view reads naturally -->
						<video bind:this={video_el} class="preview flush mirrored" autoplay playsinline muted
						></video>
						<div class="p_md">
							<p class="text_50">{video_track_info.label}</p>
							<p class="font_family_mono font_size_sm">
								{video_size || '…'}{#if fps !== null}
									· {fps} fps measured
								{/if}{#if brightness !== null}
									· brightness {Math.round(brightness * 100)}%
								{/if}
							</p>
							{#if brightness !== null && brightness < DIM_BRIGHTNESS}
								<p class="font_size_sm">
									<strong class="color_e_60">dim</strong>: more light on your face helps the
									picture, and lets the camera keep its frame rate.
								</p>
							{/if}
							{#if video_error}<p class="color_c_50">{video_error}</p>{/if}
							<p class="text_50 font_size_sm">
								The size and frame rate are requests: the camera picks its nearest mode, and the
								real one shows here and under tracks. A camera that can’t keep its frame rate in dim
								light lowers it.
							</p>
						</div>
					{:else}
						<div class="preview flush preview-empty text_50">
							{#if stream}
								the camera is off
							{:else}
								<span>
									press
									<button type="button" class="inline" onclick={start} disabled={starting}>
										start
									</button>
									to see your camera
								</span>
							{/if}
						</div>
					{/if}
				</section>
				{#if audio_track_info && stream}
					<section class="panel p_md">
						<h2 class="mb_lg">mic</h2>
						<p class="text_50">{audio_track_info.label}</p>
						<MediaMic {stream} {sink_id} />
					</section>
				{/if}
			</div>
			<div class="av-column">
				<section class="panel p_md setup">
					<h2 class="mb_lg">setup</h2>
					<div class="row gap_md flex-wrap:wrap">
						{#if stream}
							<button type="button" onclick={stop}>stop</button>
						{:else}
							<button type="button" onclick={start} disabled={starting}>
								{starting ? 'starting…' : 'start'}
							</button>
						{/if}
						<label class="row gap_xs">
							<input type="checkbox" bind:checked={want_audio} disabled={!!stream} /> mic
						</label>
						<label class="row gap_xs">
							<input type="checkbox" bind:checked={want_video} disabled={!!stream} /> camera
						</label>
					</div>
					<fieldset>
						<legend>devices</legend>
						<div class="fields">
							<label>
								<div class="title">mic</div>
								<select bind:value={audio_input_id} disabled={!!stream || !want_audio}>
									<option value="">default</option>
									{#each audio_inputs as d, i (d.deviceId)}
										<option value={d.deviceId}>
											{d.label || `mic ${i + 1}`}{is_output_monitor(d)
												? ' (computer audio, not a mic)'
												: ''}
										</option>
									{/each}
								</select>
							</label>
							<label>
								<div class="title">camera</div>
								<select bind:value={video_input_id} disabled={!!stream || !want_video}>
									<option value="">default</option>
									{#each video_inputs as d, i (d.deviceId)}
										<option value={d.deviceId}>{d.label || `camera ${i + 1}`}</option>
									{/each}
								</select>
							</label>
							<label>
								<div class="title">speakers</div>
								<select bind:value={sink_id} disabled={!can_output}>
									<option value="">default</option>
									{#each audio_outputs as d, i (d.deviceId)}
										{#if d.deviceId !== 'default'}
											<option value={d.deviceId}>{d.label || `output ${i + 1}`}</option>
										{/if}
									{/each}
								</select>
							</label>
						</div>
						{#if !can_output}
							<p class="text_50 font_size_sm">
								This browser plays through the system’s default output only.
							</p>
						{:else if can_pick_output}
							<p class="font_size_sm">
								Not listed?
								<button type="button" class="inline" onclick={pick_output}>choose an output</button>
							</p>
						{/if}
						{#if output_error}<p class="color_c_50">{output_error}</p>{/if}
						<MediaSpeakerTest {sink_id} />
					</fieldset>
					<fieldset>
						<legend>camera</legend>
						<div class="fields">
							<label>
								<div class="title">size</div>
								<select
									bind:value={resolution_index}
									onchange={apply_video_constraints}
									disabled={!want_video}
								>
									<option value={-1}>camera’s choice</option>
									{#each RESOLUTIONS as r, i (r.label)}
										<option value={i}>{r.label}</option>
									{/each}
								</select>
							</label>
							<label>
								<div class="title">shape</div>
								<select
									bind:value={aspect_index}
									onchange={apply_video_constraints}
									disabled={!want_video}
								>
									<option value={-1}>camera’s choice</option>
									{#each ASPECT_RATIOS as a, i (a.label)}
										<option value={i}>{a.label}</option>
									{/each}
								</select>
							</label>
							<label>
								<div class="title">frame rate</div>
								<select
									bind:value={frame_rate}
									onchange={apply_video_constraints}
									disabled={!want_video}
								>
									<option value={0}>camera’s choice</option>
									{#each FRAME_RATES as rate (rate)}
										<option value={rate}>{rate} fps</option>
									{/each}
								</select>
							</label>
						</div>
					</fieldset>
					<fieldset>
						<legend>mic processing</legend>
						<div class="row gap_lg flex-wrap:wrap">
							<label class="row">
								<input type="checkbox" bind:checked={echo_cancellation} disabled={!!stream} /> echo
								cancellation
							</label>
							<label class="row">
								<input type="checkbox" bind:checked={noise_suppression} disabled={!!stream} /> noise
								suppression
							</label>
							<label class="row">
								<input type="checkbox" bind:checked={auto_gain_control} disabled={!!stream} /> auto
								gain
							</label>
						</div>
						<p class="text_50 font_size_sm">These take effect on the next start.</p>
					</fieldset>
					{#if error}<p class="color_c_50">{error}</p>{/if}
				</section>
				{#if stream}
					<section class="panel p_md">
						<h2 class="mb_lg">record and play back</h2>
						<MediaRecording {stream} {sink_id} />
					</section>
					<section class="panel p_md">
						<h2 class="mb_lg">tracks</h2>
						<div class="row gap_xl flex-wrap:wrap align-items:start">
							{#if audio_track_info}{@render track_table(audio_track_info)}{/if}
							{#if video_track_info}{@render track_table(video_track_info)}{/if}
						</div>
					</section>
				{/if}
				<section class="panel p_md">
					<h2 class="mb_lg">screen share</h2>
					{#if screen_stream}
						<video bind:this={screen_el} class="preview" autoplay playsinline muted></video>
						<button type="button" onclick={stop_screen}>stop sharing</button>
						{#if screen_info}{@render track_table(screen_info)}{/if}
					{:else}
						<button type="button" onclick={start_screen}>share a screen or window</button>
					{/if}
					{#if screen_error}<p class="color_c_50">{screen_error}</p>{/if}
				</section>
				{#if stream}
					<section class="panel p_md">
						<h2 class="mb_lg">WebRTC loopback</h2>
						<MediaLoopback {stream} {sink_id} />
					</section>
				{/if}

				<section class="panel p_md">
					<h2 class="mb_lg">browser</h2>
					<table class="font_size_sm">
						<tbody>
							<tr>
								<th>secure page</th>
								<td>{secure ? 'yes' : 'no'}</td>
							</tr>
							<tr>
								<th>mic permission</th>
								<td>{mic_permission}</td>
							</tr>
							<tr>
								<th>camera permission</th>
								<td>{camera_permission}</td>
							</tr>
							<tr>
								<th>screen share</th>
								<td>{has_display_media ? 'yes' : 'no'}</td>
							</tr>
							<tr>
								<th>WebRTC</th>
								<td>{has_webrtc ? 'yes' : 'no'}</td>
							</tr>
							{#each browser_rows as [key, value] (key)}
								<tr>
									<th>{key}</th>
									<td>{value}</td>
								</tr>
							{/each}
							{#if stream && audio_input_id}
								<tr>
									<th>using mic</th>
									<td>{selected_label(audio_inputs, audio_input_id)}</td>
								</tr>
							{/if}
							{#if stream && video_input_id}
								<tr>
									<th>using camera</th>
									<td>{selected_label(video_inputs, video_input_id)}</td>
								</tr>
							{/if}
						</tbody>
					</table>
					<h3 class="mb_lg">devices</h3>
					{#if !devices.length}
						<p class="text_50">none found</p>
					{:else}
						<ul class="font_size_sm">
							{#each [...audio_inputs, ...video_inputs, ...audio_outputs] as d, i (d.kind +
								d.deviceId +
								i
							)}
								<li>
									<span class="text_50">{d.kind}</span>
									{d.label || '(name hidden until a permission is granted)'}
								</li>
							{/each}
						</ul>
					{/if}
				</section>
			</div>
		</div>
	</div>
{/if}

{#snippet track_table(info: TrackInfo)}
	<div>
		<table class="font_size_sm">
			<caption class="text-align:left">{info.label}</caption>
			<tbody>
				<tr>
					<th>state</th>
					<td>{info.state}{info.muted ? ' (muted by the source)' : ''}</td>
				</tr>
				{#each info.settings as [key, value] (key)}
					<tr>
						<th>{key}</th>
						<td class="font_family_mono">{value}</td>
					</tr>
				{/each}
			</tbody>
		</table>
		{#if info.capabilities.length}
			<details>
				<summary class="font_size_sm">what it can do</summary>
				<table class="font_size_sm">
					<tbody>
						{#each info.capabilities as [key, value] (key)}
							<tr>
								<th>{key}</th>
								<td class="font_family_mono">{value}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</details>
		{/if}
	</div>
{/snippet}

<style>
	/* two columns once the page is wide enough, one below; the page scrolls */
	.av-layout {
		container-type: inline-size;
	}
	.av-grid {
		display: grid;
		gap: var(--space_lg);
		align-items: start;
	}
	@container (min-width: 1100px) {
		.av-grid {
			grid-template-columns: 1fr 1fr;
		}
	}
	.av-column {
		display: flex;
		flex-direction: column;
		gap: var(--space_lg);
		min-width: 0;
	}
	/* the column's gap spaces the panels */
	.av-column > :global(section) {
		margin: 0;
	}
	.av-column h2 {
		font-size: var(--font_size_lg);
		margin-top: 0;
	}
	.setup fieldset {
		gap: var(--space_md);
		margin-top: var(--space_xl);
		padding: 0;
	}
	.setup legend {
		font-size: var(--font_size_md);
		font-weight: 600;
		margin-bottom: var(--space_sm);
	}
	.setup :global(p) {
		margin: 0;
	}
	/* the selects share a row while there's room, each wide enough to read */
	.fields {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(100%, 12rem), 1fr));
		gap: var(--space_md);
	}
	.av-column h3 {
		font-size: var(--font_size_md);
	}
	.preview {
		display: block;
		width: 100%;
		max-height: 50vh;
		border-radius: var(--border_radius_xs);
	}
	.camera-panel {
		overflow: hidden;
	}
	.flush {
		border-radius: 0;
	}
	.preview-empty {
		display: flex;
		align-items: center;
		justify-content: center;
		aspect-ratio: 16 / 9;
		background: var(--ink_10);
	}
	.mirrored {
		transform: scaleX(-1);
	}
</style>
