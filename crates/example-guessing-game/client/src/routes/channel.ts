export interface Channel {
	send(msg: any): Promise<void>;
	receive(): Promise<any>;
}

const MAX_U32: number = Math.pow(2, 32) - 1;

export async function connect(path: string): Promise<Channel> {
	const ws = new WebSocket(path);
	let nextId = 0;

	type SendReq = { id: number; res: (v: any) => void; rej: (v: any) => void };
	const sendQueue: SendReq[] = [];
	const recvReqQueue: ((v: any) => void)[] = [];
	const recvQueue: { id: number; val: any }[] = [];

	ws.onmessage = (msg) => {
		type Frame = { type: 'msg'; data: { id: number; val: any } } | { type: 'ack'; data: number };
		const frame: Frame = JSON.parse(msg.data);
		console.log('frame: ', frame);

		switch (frame.type) {
			case 'msg':
				if (recvReqQueue.length > 0) {
					const { id, val } = frame.data;
					const res = recvReqQueue.shift()!;
					console.log('chan recv fast and sending ack id: ', id);
					ws.send(JSON.stringify({ type: 'ack', data: id }));
					res(val);
				} else {
					console.log('no recv waiter');
					recvQueue.push(frame.data);
				}
				break;
			case 'ack':
				const id = frame.data;
				let sendReq: SendReq | undefined;
				while ((sendReq = sendQueue.shift())) {
					if (sendReq.id === id) {
						sendReq.res(null);
						return;
					} else {
						sendReq.rej(new Error(`channel ack id skipped: expect ${sendReq.id}, got ${id}`));
					}
				}
				console.error(`channel ack unknown id received: ${id}`);
				break;
		}
	};

	await new Promise((res) => (ws.onopen = res));

	return {
		async send(msg: any) {
			const id = nextId;
			nextId += 1;
			if (nextId > MAX_U32) {
				throw new Error('channel msg id u32 overflowed');
			}

			await new Promise((res, rej) => {
				console.log('chan sending msg', msg, ', id: ', id);
				ws.send(JSON.stringify({ type: 'msg', data: { id, val: msg } }));
				sendQueue.push({ id, res, rej });
			});
		},
		receive() {
			return new Promise((res) => {
				if (recvQueue.length > 0) {
					const { id, val } = recvQueue.shift()!;
					console.log('chan sending ack, id: ', id);
					ws.send(JSON.stringify({ type: 'ack', data: id }));
					res(val);
				} else {
					recvReqQueue.push(res);
				}
			});
		}
	};
}
