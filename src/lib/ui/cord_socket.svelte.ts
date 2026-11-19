/**
 * cord's WebSocket on `/api/ws`: open while an account is signed in, the only
 * way that account's requests reach the server, and the session re-check when
 * the server closes it. This module doc is where the socket's whole story
 * lives; `ui/rpc.ts`, the root layout, and the repo's docs point here.
 *
 * **Routing.** `CordSocket` is the connection `ui/rpc.ts` hands fuz_app's
 * WebSocket transport (with `queue: true`), which takes a request while the
 * connection is open or `will_reconnect`, and hands the rest to HTTP. While
 * an account is signed in, in a browser, `will_reconnect` holds, so every
 * request comes here and none falls back to HTTP, even while the socket is
 * down; signed out, it's false, and requests go over HTTP.
 *
 * **Lifecycle.** The root layout calls `set_account` from the same account
 * id its reset effect follows: the socket opens once an account is signed
 * in, closes on sign-out, and is replaced on an account change. A request
 * made before that effect runs opens it itself (`get_account_id`). Effects
 * don't run while prerendering, and off the browser nothing connects, so no
 * socket opens there. It holds one `FrontendWebsocketClient` per signed-in
 * account and swaps it rather than reusing it: a client closed as revoked
 * (`WS_CLOSE_SESSION_REVOKED`, 4001) never opens again, so a later sign-in,
 * or a session found still good, needs a fresh one. The transport registers
 * its handlers here once, and they follow each client. fuz_app's client
 * sends a `heartbeat` once either direction has been idle for
 * `DEFAULT_HEARTBEAT_INTERVAL`, and reconnects with backoff after any close
 * but a 4001 (terminal) or a 4004 (below).
 *
 * **Waiting requests.** A request made while the socket is down
 * (connecting after a sign-in, a server restart, a dropped connection)
 * waits, in the client's own queue, for the socket to open, at most
 * `CORD_SOCKET_WAIT_TIMEOUT`, then fails as a `timeout`, unsent
 * (`CORD_SOCKET_WAIT_TIMEOUT_REASON`); one made while the socket is closed
 * as revoked waits on the re-check first. A sign-out or an account change
 * (`CORD_SOCKET_ACCOUNT_CHANGED_REASON`), even a sign-out and back in as the
 * same account, drops what's waiting, so nothing waiting is sent on the next
 * session's socket (a reconnect, like HTTP, goes as whoever the browser's
 * cookie now names). A request already sent when the connection drops, a
 * sign-out's or an account change's included, fails with
 * `CORD_SOCKET_LOST_REASON`: the server may or may not have run it,
 * whatever the close. That includes a 4001, which can
 * only close a socket that opened, and opening sends everything queued. The
 * reasons and their words live in `ui/cord_errors.ts`, and these errors
 * carry the words as their own messages.
 *
 * **Superseded.** A socket the server closed to admit a newer one on the
 * account (`WS_CLOSE_CONNECTION_LIMIT`, 4004, `superseded`) stays closed
 * until something calls `connect`. The next request does, since a request is
 * the user acting; reopening may close another tab's oldest socket in turn,
 * which likewise reopens only on its own next request, so the tabs never
 * close each other in a loop. The role gate's close
 * (`WS_CLOSE_PERMISSION_REVOKED`, 4006) can't reach cord: `/api/ws` mounts
 * no required roles.
 *
 * **The session re-check.** A 4001 means the session was revoked or expired
 * while the socket was open, but not always this tab's session: a
 * revocation that ends every session of the account (a logout elsewhere)
 * closes every socket on it, whichever session is still live elsewhere. And
 * an upgrade the server refuses for a dead session reads to the browser as a
 * dropped connection, so a session that expired while the socket was down
 * shows only as reconnects that keep failing
 * (`CORD_SOCKET_RECHECK_RECONNECTS`). Either one puts the session in doubt,
 * and the root layout calls `recheck_session_if_in_doubt` from an `$effect`;
 * `recheck_session` settles it by probing `/api/account/status`. Only a
 * definitive 401, or a live session for another account, runs the session
 * check (`end_session`, the auth state's `check_session`), which signs the
 * tab out as a logout would (the root layout's reset: the cords, spaces, and
 * tabs, and the rail signed out), or moves it to whoever the browser is now
 * signed in as, without reconnecting, though the `on_logout` listeners don't
 * run; the socket follows the account it leaves. Anything else keeps the session,
 * reopening a revoked socket on a fresh client. A network failure or a 5xx
 * is not a 401, so a server restart or an outage never signs anyone out. A
 * re-check answers only for the client it began on, and its probe gives up
 * as unknown after `CORD_SOCKET_WAIT_TIMEOUT`. A socket revoked again soon
 * after such a reopen (`CORD_SOCKET_REOPEN_WINDOW`) reopens on a backoff,
 * never in a tight loop.
 *
 * **What shows.** The rail (`CordNav`) says the socket is reconnecting once
 * it has been down for `CORD_SOCKET_RECONNECTING_DELAY`, except while it's
 * closed as superseded, which isn't trying.
 *
 * @module
 */

import { BROWSER } from 'esm-env';
import { untrack } from 'svelte';
import {
	DEFAULT_BACKOFF_FACTOR,
	DEFAULT_RECONNECT_DELAY,
	DEFAULT_RECONNECT_DELAY_MAX,
	FrontendWebsocketClient,
	type SocketStatus
} from '@fuzdev/fuz_app/actions/socket.svelte.ts';
import {
	to_same_origin_ws_url,
	type WebsocketRpcConnection
} from '@fuzdev/fuz_app/actions/transports_ws.ts';
import { JSONRPC_ERROR_CODES, jsonrpc_errors } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import type { SessionProbe } from './cord_auth_state.svelte.ts';
import {
	CORD_SIGNED_OUT_MESSAGE,
	CORD_SOCKET_ACCOUNT_CHANGED_MESSAGE,
	CORD_SOCKET_ACCOUNT_CHANGED_REASON,
	CORD_SOCKET_CANCELLED_MESSAGE,
	CORD_SOCKET_LOST_MESSAGE,
	CORD_SOCKET_LOST_REASON,
	CORD_SOCKET_NO_SOCKET_MESSAGE,
	CORD_SOCKET_WAIT_TIMEOUT_MESSAGE,
	CORD_SOCKET_WAIT_TIMEOUT_REASON,
	rpc_error_code
} from './cord_errors.ts';

/** The WebSocket endpoint `cordd` mounts, beside `/api/rpc`. */
export const CORD_WS_PATH = '/api/ws';

/**
 * Failed reconnects in a row after which the session is in doubt. The first
 * failure is usually a server restart or a network blip; a second makes a
 * refused upgrade likely enough to ask.
 */
export const CORD_SOCKET_RECHECK_RECONNECTS = 2;

/**
 * How long a signed-in request waits for the socket to open, in ms, before
 * it fails as a `timeout`, unsent. Long enough to ride out a server restart
 * under the client's reconnect backoff.
 */
export const CORD_SOCKET_WAIT_TIMEOUT = 30_000;

/**
 * A socket revoked again within this many ms of a reopen after a revoke
 * reopens on a backoff (the client's reconnect delays) rather than at once.
 */
export const CORD_SOCKET_REOPEN_WINDOW = 30_000;

/** How long the socket is down, in ms, before the rail says it's reconnecting. */
export const CORD_SOCKET_RECONNECTING_DELAY = 2_000;

/** The options a socket request takes: `WebsocketRpcConnection`'s. */
export type CordSocketRequestOptions = NonNullable<
	Parameters<WebsocketRpcConnection['request']>[2]
>;

/** What `CordSocket` needs of a WebSocket client: `FrontendWebsocketClient`'s surface. */
export type CordSocketClient = Omit<WebsocketRpcConnection, 'will_reconnect'> &
	Pick<
		FrontendWebsocketClient,
		'status' | 'revoked' | 'superseded' | 'reconnect_count' | 'connect' | 'disconnect'
	>;

/** The clock and timers `CordSocket` waits on. */
export interface CordSocketTimers {
	now: () => number;
	set_timeout: (callback: () => void, ms: number) => unknown;
	clear_timeout: (handle: unknown) => void;
}

const default_timers: CordSocketTimers = {
	now: () => Date.now(),
	set_timeout: (callback, ms) => setTimeout(callback, ms),
	clear_timeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>)
};

export interface CordSocketOptions {
	/**
	 * Asks the server whether the session is live and still this account's,
	 * without changing any state. `signal` aborts it after the wait timeout,
	 * and a probe still unanswered then reads as `unknown`.
	 */
	probe_session: (account_id: string, signal: AbortSignal) => Promise<SessionProbe>;
	/**
	 * Settles the session in this tab once the server says it's gone or
	 * someone else's: the auth state's `check_session`.
	 */
	end_session: () => Promise<void>;
	/**
	 * The account signed in now, read when routing a request and after
	 * `end_session`, so the socket follows the session without waiting for
	 * the layout's `set_account`. Defaults to the last `set_account`.
	 */
	get_account_id?: () => string | null;
	/** Builds the client for one sign-in. Defaults to a `FrontendWebsocketClient` on `/api/ws`. */
	create_client?: () => CordSocketClient;
	/** Whether this runs in a browser; off it, nothing connects. Defaults to `esm-env`'s `BROWSER`. */
	browser?: boolean;
	/** Defaults to the global clock and timers. */
	timers?: CordSocketTimers;
	/** Bounds a request's wait and a session probe. Defaults to `CORD_SOCKET_WAIT_TIMEOUT`. */
	wait_timeout?: number;
}

/** One sign-in's session, and its reopens after a revoke. */
interface CordSocketSession {
	readonly account_id: string;
	/** When the socket last reopened after a revoke, or `null` if it hasn't. */
	reopened_at: number | null;
	/** How many reopens after a revoke came quickly in a row. */
	reopen_streak: number;
}

const create_default_client = (): CordSocketClient =>
	new FrontendWebsocketClient(to_same_origin_ws_url(CORD_WS_PATH));

/** The backoff for the `streak`th quick revoke in a row, on the client's own reconnect schedule. */
const to_reopen_delay = (streak: number): number =>
	streak <= 0
		? 0
		: Math.round(
				Math.min(
					DEFAULT_RECONNECT_DELAY_MAX,
					DEFAULT_RECONNECT_DELAY * DEFAULT_BACKOFF_FACTOR ** (streak - 1)
				)
			);

const to_lost_error = (): Error =>
	jsonrpc_errors.service_unavailable(CORD_SOCKET_LOST_MESSAGE, { reason: CORD_SOCKET_LOST_REASON });

/**
 * The signed-in account's socket. The root layout calls `set_account` as the
 * account changes and `recheck_session_if_in_doubt` from an `$effect`.
 */
export class CordSocket implements WebsocketRpcConnection {
	readonly #probe_session: (account_id: string, signal: AbortSignal) => Promise<SessionProbe>;
	readonly #end_session: () => Promise<void>;
	readonly #get_account_id: (() => string | null) | null;
	readonly #create_client: () => CordSocketClient;
	readonly #browser: boolean;
	readonly #timers: CordSocketTimers;
	readonly #wait_timeout: number;

	#client: CordSocketClient | null = $state.raw(null);
	/**
	 * The signed-in account's session, `null` signed out. Each real
	 * `set_account` change replaces it, a sign-out included, so a request can
	 * tell its session ended even when the same account is back.
	 */
	#session: CordSocketSession | null = null;
	#unsubscribe_client: (() => void) | null = null;
	/** The recheck in flight, and the client it's for: only a recheck for the current client is shared. */
	#recheck: { client: CordSocketClient; promise: Promise<void> } | null = null;
	/** A fresh client's `connect`, held back on a backoff. */
	#connect_timer: unknown = null;

	/** Clients a sign-out or account change dropped while open: all they held was sent. */
	readonly #dropped_open: WeakSet<CordSocketClient> = new WeakSet();

	readonly #message_handlers: Set<(event: MessageEvent) => void> = new Set();
	readonly #error_handlers: Set<(event: Event) => void> = new Set();

	/** The client's status; `initial` with no account signed in, and for a reopen held on its backoff. */
	readonly status: SocketStatus = $derived(this.#client?.status ?? 'initial');
	readonly connected: boolean = $derived(this.#client?.connected ?? false);
	/** Whether the server closed the socket as revoked (4001). It stays closed until `recheck_session` reopens it. */
	readonly revoked: boolean = $derived(this.#client?.revoked ?? false);
	/**
	 * Whether the server closed the socket to admit a newer one on the
	 * account (4004). It stays closed, not trying, until the next request
	 * reopens it.
	 */
	readonly superseded: boolean = $derived(this.#client?.superseded ?? false);
	/** Failed reconnects in a row since the socket was last open. */
	readonly reconnect_count: number = $derived(this.#client?.reconnect_count ?? 0);

	constructor(options: CordSocketOptions) {
		this.#probe_session = options.probe_session;
		this.#end_session = options.end_session;
		this.#get_account_id = options.get_account_id ?? null;
		this.#create_client = options.create_client ?? create_default_client;
		this.#browser = options.browser ?? BROWSER;
		this.#timers = options.timers ?? default_timers;
		this.#wait_timeout = options.wait_timeout ?? CORD_SOCKET_WAIT_TIMEOUT;
	}

	/**
	 * True while an account is signed in, in a browser: its socket is open or
	 * coming back, or its session is being settled. fuz_app's transport
	 * queues a request here while this holds, so a signed-in request waits
	 * for the socket rather than going over HTTP; signed out, it's false, and
	 * requests go over HTTP.
	 */
	get will_reconnect(): boolean {
		return this.#browser && this.#current_account_id() !== null;
	}

	/**
	 * Follow the signed-in account: a socket opens for an account, closes
	 * when it signs out, and is replaced when another signs in, which drops
	 * every request waiting on the old one. The same account again changes
	 * nothing. Reads nothing trackably, so an `$effect` that calls it reruns
	 * only on what it passes.
	 *
	 * @param account_id - the signed-in account's id, or `null` when signed out
	 * @mutates this - replaces or drops the client
	 */
	set_account(account_id: string | null): void {
		if (account_id === (this.#session?.account_id ?? null)) return;
		this.#session =
			account_id === null ? null : { account_id, reopened_at: null, reopen_streak: 0 };
		untrack(() => {
			this.#close();
			if (account_id !== null) this.#open(0);
		});
	}

	/**
	 * Recheck the session if the server may have ended it: it closed the
	 * socket as revoked, or reconnects keep failing
	 * (`CORD_SOCKET_RECHECK_RECONNECTS`). It reads the client's state, so an
	 * `$effect` that calls it runs again as that changes, and each further
	 * failed reconnect asks again.
	 */
	recheck_session_if_in_doubt(): void {
		if (this.revoked || this.reconnect_count >= CORD_SOCKET_RECHECK_RECONNECTS) {
			void untrack(() => this.recheck_session());
		}
	}

	/**
	 * Ask the server whether the session is still live and this account's. A
	 * 401, or another account's session, runs `end_session` and follows the
	 * account it leaves signed in; anything else keeps the session, and a
	 * socket closed as revoked reopens on a fresh client. Calls while one is
	 * in flight for the same client share it; an answer that arrives after
	 * the client was replaced is dropped.
	 */
	recheck_session(): Promise<void> {
		const client = this.#client;
		if (!client) return Promise.resolve();
		if (this.#recheck?.client === client) return this.#recheck.promise;
		const entry: { client: CordSocketClient; promise: Promise<void> } = {
			client,
			promise: this.#run_recheck(client).finally(() => {
				if (this.#recheck === entry) this.#recheck = null;
				// the client this was for has been replaced; if its successor was
				// revoked meanwhile, make sure its doubt is settled too
				if (client !== this.#client && this.#client?.revoked) void this.recheck_session();
			})
		};
		this.#recheck = entry;
		return entry.promise;
	}

	async #run_recheck(client: CordSocketClient): Promise<void> {
		const session = this.#session;
		if (!session) return;
		// bounded, so a hung probe can't leave a revoked socket unreopened
		const controller = new AbortController();
		const timer = this.#timers.set_timeout(() => controller.abort(), this.#wait_timeout);
		const gave_up = new Promise<SessionProbe>((resolve) => {
			controller.signal.addEventListener('abort', () => resolve('unknown'), { once: true });
		});
		let probe: SessionProbe;
		try {
			probe = await Promise.race([
				this.#probe_session(session.account_id, controller.signal),
				gave_up
			]);
		} catch {
			probe = 'unknown';
		} finally {
			this.#timers.clear_timeout(timer);
		}
		if (client !== this.#client) return;
		if (probe === 'invalid' || probe === 'changed') {
			try {
				await this.#end_session();
			} catch (error) {
				console.error('[cord_socket] settling the session failed:', error);
			}
			// follow whoever the check left signed in: no one, or another account,
			// replaces the client; the same account keeps it
			this.#follow_account();
			if (client !== this.#client) return;
		}
		// Still live, or the server couldn't say: a revoked client never opens
		// again, so replace it. If the session is in fact gone, the new upgrade
		// is refused, the reconnects fail, and the doubt comes back.
		if (client.revoked) this.#reopen_after_revoke(session);
	}

	/** @mutates session - counts the reopen toward its quick-revoke streak */
	#reopen_after_revoke(session: CordSocketSession): void {
		const now = this.#timers.now();
		session.reopen_streak =
			session.reopened_at !== null && now - session.reopened_at < CORD_SOCKET_REOPEN_WINDOW
				? session.reopen_streak + 1
				: 0;
		session.reopened_at = now;
		this.#open(to_reopen_delay(session.reopen_streak));
	}

	#current_account_id(): string | null {
		return this.#get_account_id ? this.#get_account_id() : (this.#session?.account_id ?? null);
	}

	#follow_account(): void {
		if (this.#get_account_id) this.set_account(this.#get_account_id());
	}

	/** Open a fresh client, connecting it after `delay` ms; requests queue on it meanwhile. */
	#open(delay: number): void {
		this.#close();
		if (!this.#browser) return;
		const client = this.#create_client();
		const remove_message = client.add_message_handler((event) => {
			for (const handler of this.#message_handlers) handler(event);
		});
		const remove_error = client.add_error_handler((event) => {
			for (const handler of this.#error_handlers) handler(event);
		});
		this.#unsubscribe_client = () => {
			remove_message();
			remove_error();
		};
		this.#client = client;
		if (delay <= 0) {
			client.connect();
			return;
		}
		this.#connect_timer = this.#timers.set_timeout(() => {
			this.#connect_timer = null;
			if (this.#client === client) client.connect();
		}, delay);
	}

	#close(): void {
		if (this.#connect_timer !== null) {
			this.#timers.clear_timeout(this.#connect_timer);
			this.#connect_timer = null;
		}
		this.#unsubscribe_client?.();
		this.#unsubscribe_client = null;
		const client = this.#client;
		if (!client) return;
		this.#client = null;
		// an open client holds nothing unsent (opening sent its queue), so what
		// the disconnect rejects was on the wire, its outcome unknown
		if (client.connected) this.#dropped_open.add(client);
		client.disconnect();
	}

	send(data: object): boolean {
		return this.#client?.send(data) ?? false;
	}

	/**
	 * Send a request over the socket. While it's open, at once; while it's
	 * down, once it opens, failing as a `timeout` after the wait timeout;
	 * while it's closed as revoked, once the re-check settles; while it's
	 * closed as superseded, once it reopens, which this starts. Signed out,
	 * or once the session it was made in ends before it's sent, it fails as
	 * `unauthenticated`, unsent; one already sent when the connection drops,
	 * a sign-out's or an account change's included, fails as lost
	 * (`CORD_SOCKET_LOST_REASON`).
	 */
	async request(
		method: string,
		params: unknown,
		options: CordSocketRequestOptions = {}
	): Promise<unknown> {
		this.#follow_account();
		const session = this.#session;
		let client = this.#client;
		if (!client || !session) {
			throw this.#browser && !session
				? jsonrpc_errors.unauthenticated(CORD_SIGNED_OUT_MESSAGE)
				: jsonrpc_errors.service_unavailable(CORD_SOCKET_NO_SOCKET_MESSAGE);
		}
		const { account_id } = session;
		if (client.connected) {
			return this.#send(client, account_id, method, params, options);
		}
		// closed to admit a newer socket, nothing reopens it on its own
		if (client.superseded) client.connect();

		// wait, bounded: the timeout aborts the request only while it's still
		// waiting, since once the socket opens the client has sent it
		const controller = new AbortController();
		const caller_signal = options.signal;
		const abort = (): void => controller.abort();
		if (caller_signal?.aborted) controller.abort();
		else caller_signal?.addEventListener('abort', abort, { once: true });
		const aborted = new Promise<void>((resolve) => {
			controller.signal.addEventListener('abort', () => resolve(), { once: true });
		});
		let waiting_on: CordSocketClient | null = null;
		let timed_out = false;
		const timer = this.#timers.set_timeout(() => {
			if (waiting_on?.connected) return;
			timed_out = true;
			controller.abort();
		}, this.#wait_timeout);
		const to_abort_error = () =>
			timed_out
				? jsonrpc_errors.timeout(CORD_SOCKET_WAIT_TIMEOUT_MESSAGE, {
						reason: CORD_SOCKET_WAIT_TIMEOUT_REASON
					})
				: jsonrpc_errors.request_cancelled(CORD_SOCKET_CANCELLED_MESSAGE);
		try {
			if (client.revoked) {
				await Promise.race([this.recheck_session(), aborted]);
				if (controller.signal.aborted) throw to_abort_error();
				const next: CordSocketClient | null = this.#client;
				// the session it was made in ended meanwhile, even if the same
				// account is back: never send it in the session after
				if (!next || this.#session !== session || next.revoked) {
					throw this.#to_session_ended_error(account_id);
				}
				client = next;
			}
			waiting_on = client;
			return await this.#send(client, account_id, method, params, {
				...options,
				signal: controller.signal
			});
		} catch (error) {
			if (timed_out) throw to_abort_error();
			throw error;
		} finally {
			this.#timers.clear_timeout(timer);
			caller_signal?.removeEventListener('abort', abort);
		}
	}

	/** Send `method` on `client` and classify how it failed. */
	async #send(
		client: CordSocketClient,
		account_id: string,
		method: string,
		params: unknown,
		options: CordSocketRequestOptions
	): Promise<unknown> {
		try {
			return await client.request(method, params, options);
		} catch (error) {
			const code = rpc_error_code(error);
			if (code === JSONRPC_ERROR_CODES.service_unavailable) {
				// the client was dropped on a sign-out or an account change: what
				// an open one held was sent, and what a closed one held never was
				if (client !== this.#client) {
					throw this.#dropped_open.has(client)
						? to_lost_error()
						: this.#to_session_ended_error(account_id);
				}
				// the connection closed under it: a frame the server sends back
				// arrives while connected, so this one never got its answer
				if (!client.connected) throw to_lost_error();
			}
			// A 4001 rejects everything on the client, yet one session's
			// revocation can close every socket on the account, so it says
			// nothing yet about this tab's session: the re-check settles that.
			// It can only close a socket that opened, and opening sent all that
			// was queued (and `request` hands a revoked client nothing), so
			// whatever it rejects was sent, with its outcome unknown.
			if (code === JSONRPC_ERROR_CODES.unauthenticated && client.revoked) throw to_lost_error();
			throw error;
		}
	}

	/** Signed out, or, when another account is signed in now, the account changed. */
	#to_session_ended_error(account_id: string): Error {
		const current = this.#session?.account_id ?? null;
		return current !== null && current !== account_id
			? jsonrpc_errors.unauthenticated(CORD_SOCKET_ACCOUNT_CHANGED_MESSAGE, {
					reason: CORD_SOCKET_ACCOUNT_CHANGED_REASON
				})
			: jsonrpc_errors.unauthenticated(CORD_SIGNED_OUT_MESSAGE);
	}

	add_message_handler(handler: (event: MessageEvent) => void): () => void {
		this.#message_handlers.add(handler);
		return () => {
			this.#message_handlers.delete(handler);
		};
	}

	add_error_handler(handler: (event: Event) => void): () => void {
		this.#error_handlers.add(handler);
		return () => {
			this.#error_handlers.delete(handler);
		};
	}
}
