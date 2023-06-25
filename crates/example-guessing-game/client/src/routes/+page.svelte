<script lang="ts" context="module">
	const encoder = new TextEncoder();
</script>

<script lang="ts">
	import { onMount } from 'svelte';

	import { type Channel, connect } from './channel';

	const WS_HOST = 'localhost:5173';

	let room = '';
	let workerLock: Int32Array | undefined;
	let inputArray: Uint8Array | undefined;
	let chan: Channel | undefined;
	let error = '';
	let roomCreated = false;
	let canSessionStart = false;
	let canGuess = false;
	let isDragging = false;
	let guess = 50;

	const playerCandidates = ['red', 'fuchsia', 'green', 'lime', 'yellow', 'blue', 'aqua', 'orange'];
	type Player = (typeof playerCandidates)[number];
	let currentPlayer: Player = 'red';

	type State = {
		turns: Turn[];
		winner: Player | null;
	};
	type Turn = { player: Player; guess: number | null; result: Ordering | null };
	type Ordering = 'Less' | 'Equal' | 'Greater';

	let state: State | undefined;

	function sendInput(input: any) {
		if (!workerLock || !inputArray) {
			return;
		}

		const inputBytes = encoder.encode(JSON.stringify(input));
		workerLock[0] = inputBytes.length;
		inputArray.set(inputBytes);

		Atomics.notify(workerLock, 0);
	}

	onMount(async () => {
		const worker = new Worker(new URL('./logic-worker.ts', import.meta.url));
		type Msg =
			| { type: 'shmem'; lock: Int32Array; inputArray: Uint8Array }
			| { type: 'io'; output: any };

		const msgBuffer: Msg[] = [];
		let msgNotify = () => {};

		worker.onmessage = (msg: MessageEvent<Msg>) => {
			msgBuffer.push(msg.data);
			msgNotify();
		};
		const nextMsg = () =>
			new Promise((res: (m: Msg) => void) => {
				let msg;
				if ((msg = msgBuffer.shift())) {
					res(msg);
				} else {
					msgNotify = () => res(msgBuffer.shift()!);
				}
			});

		let sessionEnd = false;
		while (!sessionEnd) {
			const msg = await nextMsg();

			if (msg.type == 'shmem' && !workerLock) {
				workerLock = msg.lock;
				inputArray = msg.inputArray;
			} else if (msg.type == 'io' && workerLock) {
				type Output =
					| { type: 'error'; data: string }
					| { type: 'sessionStart' }
					| { type: 'sessionEnd' }
					| { type: 'updateState'; data: State }
					| { type: 'doTaskIf'; data: { allowed: Player[] } }
					| { type: 'taskDone'; data: { targets: Player[]; value: any } }
					| { type: 'random'; data: { start: number; end: number } }
					| { type: 'action'; data: { from: Player; param: Action } };
				type Action = 'Guess';

				const output: Output = msg.output;
				console.log('Output: ', output);

				switch (output.type) {
					case 'error':
						error = output.data;
						workerLock = undefined;
						break;
					case 'sessionStart':
						console.log('sess start');
						canSessionStart = true;
						break;
					case 'sessionEnd':
						sessionEnd = true;
						break;
					case 'updateState':
						state = output.data;
						console.log('new state: ', state);
						sendInput(null);
						break;
					case 'doTaskIf':
						if (output.data.allowed.includes(currentPlayer)) {
							sendInput({ type: 'doTask' });
						} else {
							sendInput(await chan?.receive());
						}
						break;
					case 'taskDone':
						sendInput(null);
						await chan?.receive();
						break;
					case 'random':
						function randomRange(start: number, end: number): number {
							const rand = Math.random();
							if (rand === 1 || start > end) {
								// just in case
								return start;
							}
							const range = end - start + 1;
							return Math.floor(rand * range) + start;
						}
						sendInput(randomRange(output.data.start, output.data.end));
						break;
					case 'action':
						console.log('action: ', output.data.param);

						if (output.data.from != currentPlayer) {
							sendInput(await chan!.receive());
						} else {
							canSubmitGuess = true;

							switch (output.data.param) {
								case 'Guess':
									canGuess = true;
									break;
							}
						}
						break;
				}
			} else {
				console.error('unknown msg type: ', msg.type, ', worker: ', workerLock);
			}
		}
	});

	async function onCreate() {
		canSessionStart = false;

		const createResp = await fetch(`/room`, {
			method: 'POST',
			headers: { 'Content-Type': 'application/json' },
			body: '{"game":"guessing_game"}'
		});
		const createBody = await createResp.json();
		room = createBody.room;
		chan = await connect(`ws://${WS_HOST}/room/${room}/connect?color=${currentPlayer}`);

		roomCreated = true;

		const info: { room: any } = await chan.receive();
		console.log('info: ', info);
		sendInput(info.room);
	}

	async function onStart() {
		const startResp = await fetch(`/room/${room}/start`, {
			method: 'POST'
		});
		const startBody = await startResp.json();

		if (!startBody.ok) {
			throw new Error('game start request failed');
		}
	}

	async function onJoin() {
		canSessionStart = false;
		chan = await connect(`ws://${WS_HOST}/room/${room}/connect?color=${currentPlayer}`);

		const info: { room: any } = await chan.receive();
		console.log('info: ', info);
		sendInput(info.room);
	}

	function arrowTransform(ord: Ordering): string {
		switch (ord) {
			case 'Greater':
				return 'rotate(180 10 6)';
			case 'Less':
				return '';
			case 'Equal':
				return 'rotate(270 10 6)';
		}
	}

	function offset(rect: DOMRect, ev: TouchEvent | MouseEvent): { x: number; y: number } {
		if ('touches' in ev) {
			const { top, left } = rect;
			const { clientX, clientY } = ev.touches[0];
			return { x: clientX - left, y: clientY - top };
		} else {
			const { offsetX, offsetY } = ev;
			return { x: offsetX, y: offsetY };
		}
	}

	function onDrag(this: Element, ev: TouchEvent | MouseEvent) {
		if (!isDragging) {
			return;
		}
		const rect = this.getBoundingClientRect();
		let xy = offset(rect, ev);
		let pos = Math.round((xy.x / rect.width) * 120);
		if (pos < 10) {
			pos = 10;
		} else if (pos > 110) {
			pos = 110;
		}
		guess = pos - 10;
	}

	function onDragStart(this: Element, ev: TouchEvent | MouseEvent) {
		isDragging = true;
		onDrag.call(this, ev);
	}

	function onDragStop() {
		isDragging = false;
	}

	let canSubmitGuess = false;
	async function submitGuess() {
		canSubmitGuess = false;
		sendInput(guess);
		await chan!.send(guess);
	}
</script>

{#if !state}
	<p>
		<label>
			Select color:
			<select bind:value={currentPlayer}>
				{#each playerCandidates as player}
					<option value={player}>{player}</option>
				{/each}
			</select>
		</label>
	</p>
	<p>
		<label>
			{#if roomCreated}
				<button on:click={onStart}>Start Game</button>
			{:else}
				<button disabled={!canSessionStart} on:click={onCreate}>Create New Game</button>
			{/if}
			<button disabled={!canSessionStart} on:click={onJoin}>Join Game</button>
			<input type="text" bind:value={room} />
		</label>
	</p>
{:else}
	Room {room}
	<div class="turn-container">
		<span>You</span>
		<div class="turn-block" style:border-color={currentPlayer} />
	</div>
	<div class="turn-container">
		<span>Turn</span>
		{#each state.turns as turn (turn.player)}
			<div class="turn-block" style:border-color={turn.player} />
		{/each}
	</div>
	<div class="turn-container">
		<span>Winner</span>
		{#if state.winner}
			<div class="turn-block" style:border-color={state.winner} />
		{/if}
	</div>
	<svg
		xmlns="http://www.w3.org/2000/svg"
		viewBox="0 0 120 30"
		on:mousedown={onDragStart}
		on:touchstart={onDragStart}
		on:mouseup={onDragStop}
		on:touchend={onDragStop}
		on:mouseleave={onDragStop}
		on:mousemove={onDrag}
		on:touchmove={onDrag}
	>
		<path fill="grey" d="M 10,17 A 3 3 0 0 0 10 23 L 110,23 A 3 3 0 0 0 110 17 L 10,17" />
		{#each state.turns as turn (turn.player)}
			{#if turn.guess != null}
				<g transform={`translate(${turn.guess} 0)`}>
					<polyline fill={turn.player} points="10,17 7,12 13,12" />
					{#if turn.result != null}
						<polyline
							fill="none"
							stroke-linejoin="round"
							stroke-linecap="round"
							stroke-width="2"
							stroke={turn.player}
							points="11,9 7,6 11,3 7,6 13,6"
							transform={arrowTransform(turn.result)}
						/>
					{/if}
				</g>
			{/if}
		{/each}
		<circle transform="translate(10,20)" fill="lightgrey" cx={guess} cy="0" r="4" />
		<polyline transform={`translate(${guess} 0)`} points="10,17 7,12 13,12" />
	</svg>
	<div class="updown-container">
		<div class="updown-padding" />
		<button
			class="updown-btn"
			on:click={() => {
				if (guess > 0) guess -= 1;
			}}
		>
			DOWN
		</button>
		<div class="updown-display">{guess}</div>
		<button
			class="updown-btn"
			on:click={() => {
				if (guess < 100) guess += 1;
			}}
		>
			UP
		</button>
		<div class="updown-padding">
			<button class="submit-btn" disabled={!canSubmitGuess} on:click={submitGuess}>SUBMIT</button>
		</div>
	</div>
{/if}

{#if error}
	ERROR: {error}
{/if}

<style>
	.turn-container {
		display: flex;
	}
	.turn-container > span {
		width: 5em;
		text-align: end;
	}
	.turn-container > span::after {
		content: ' >';
	}
	.turn-block {
		margin: 0.3em;
		border: solid 0.5em;
		border-radius: 1em;
	}
	.updown-container {
		margin-top: 2em;
		display: flex;
		justify-content: center;
	}
	.updown-padding {
		flex-grow: 1;
		flex-basis: 0;
		display: flex;
	}
	.updown-btn {
		width: 4em;
	}
	.updown-display {
		width: 3em;
		text-align: center;
		margin: 0 0.5em;
		border: solid black;
		border-width: 0.2em;
		border-radius: 0.5em;
	}
	.submit-btn {
		margin-left: 1em;
	}
</style>
